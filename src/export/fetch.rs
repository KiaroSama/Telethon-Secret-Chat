//! Single-chat takeout orchestration with ordinary-request fallback and complete Desktop slicing.
use super::{
    error::ExportError,
    fetch_client::*,
    fetch_files::{MessageFiles, message_file_work},
    files::normalize_path,
    format::{DateContext, countries_from_tl},
    model::*,
    model_dialogs::*,
    model_message::*,
    model_peers::type_is,
    parse_rich::{extract_full_rich_message, parse_rich_message},
    settings::{Environment, Settings},
    writer::{ExportResult, ExportWriter},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub fn takeout_options(s: &Settings) -> Value {
    let files = s.media.types != 0 && s.media.size_limit > 0 || s.types & 0x802 != 0;
    json!({"_":"account.InitTakeoutSession","contacts":s.types&4!=0,"message_users":s.types&(Settings::PERSONAL_CHATS|Settings::BOT_CHATS)!=0,"message_chats":s.types&Settings::PRIVATE_GROUPS!=0,"message_megagroups":s.types&(Settings::PRIVATE_GROUPS|Settings::PUBLIC_GROUPS)!=0,"message_channels":s.types&(Settings::PRIVATE_CHANNELS|Settings::PUBLIC_CHANNELS)!=0,"files":files,"file_max_size":if files{Some(s.media.size_limit)}else{None}})
}
pub async fn export_single_chat<C: ExportClient>(
    client: &C,
    options: &Settings,
    writer: &mut dyn ExportWriter,
    environment: &Environment,
) -> FetchResult<ExportResult> {
    export_single_chat_with_date(client, options, writer, environment, None).await
}
/// Optional clock seam makes scheduled folder naming reproducible without changing server timestamps.
pub async fn export_single_chat_with_date<C: ExportClient>(
    client: &C,
    options: &Settings,
    writer: &mut dyn ExportWriter,
    environment: &Environment,
    today: Option<&str>,
) -> FetchResult<ExportResult> {
    let mut settings = options.normalized();
    let Some(peer_id) = settings.single_peer else {
        return Err(ExportError::InvalidOptions.into());
    };
    let dates = settings
        .time_zone
        .as_deref()
        .map_or_else(|| Ok(DateContext::system()), DateContext::named)?;
    settings.path = normalize_path(&settings, today, &dates)?;
    let input = settings.single_peer_input.clone().unwrap_or_else(|| {
        if peer_id == 0 {
            json!({"_":"InputPeerSelf"})
        } else if peer_is_chat(peer_id) {
            json!({"_":"InputPeerChat","chat_id":peer_to_chat(peer_id)})
        } else if peer_is_channel(peer_id) {
            json!({"_":"InputPeerChannel","channel_id":peer_to_channel(peer_id),"access_hash":0})
        } else {
            json!({"_":"InputPeerUser","user_id":peer_id,"access_hash":0})
        }
    });
    let users = client
        .request(
            RequestScope::default(),
            json!({"_":"users.GetUsers","id":[{"_":"InputUserSelf"}]}),
        )
        .await?;
    let self_id = users
        .as_array()
        .into_iter()
        .flatten()
        .find(|u| type_is(u, "User") && boolean(u, "is_self"))
        .map(|u| integer(u, "id"))
        .ok_or(ExportError::InvalidRecord)?;
    let mut environment = environment.clone();
    match client
        .request(
            RequestScope::default(),
            json!({"_":"help.GetCountriesList","lang_code":"","hash":0}),
        )
        .await
    {
        Ok(c) if type_is(&c, "CountriesList") || type_is(&c, "help.CountriesList") => {
            environment.countries = countries_from_tl(&c)
        }
        Err(FetchError::Rpc { .. }) | Ok(_) => {}
        Err(e) => return Err(e),
    }
    let mut files = MessageFiles::new(client, settings.clone(), self_id);
    let mut reason = String::new();
    match client
        .request(RequestScope::default(), takeout_options(&settings))
        .await
    {
        Ok(t) => files.scope.takeout = Some(integer(&t, "id")),
        Err(FetchError::Rpc { kind, .. }) => reason = kind,
        Err(FetchError::Local(ExportError::InvalidOptions)) => reason = "ValueError".into(),
        Err(e) => return Err(e),
    }
    let takeout = files.scope.takeout.is_some();
    let result = run(&mut files, &input, writer, &environment, &dates).await;
    // A successfully opened takeout is always finalized, including writer/download failures.
    let finish = if takeout {
        client
            .request(
                files.scope.clone(),
                json!({"_":"account.FinishTakeoutSession","success":result.is_ok()}),
            )
            .await
            .map(|_| ())
    } else {
        Ok(())
    };
    result?;
    finish?;
    Ok(ExportResult {
        path: settings.path,
        takeout,
        takeout_error: reason,
        messages: files_message_count(&files),
        files: files.files,
    })
}
// Counts belong to the driver, not the loaded-file count; stored locally in DriverResult below.
fn files_message_count<C: ExportClient>(files: &MessageFiles<'_, C>) -> usize {
    files.messages
}
async fn run<C: ExportClient>(
    files: &mut MessageFiles<'_, C>,
    input: &Value,
    writer: &mut dyn ExportWriter,
    environment: &Environment,
    dates: &DateContext,
) -> FetchResult<()> {
    if files.scope.takeout.is_some() {
        let splits = files.main(json!({"_":"messages.GetSplitRanges"})).await?;
        files.splits = splits
            .as_array()
            .cloned()
            .ok_or(ExportError::InvalidRecord)?;
    }
    if files.splits.is_empty() {
        files
            .splits
            .push(json!({"_":"MessageRange","min_id":1,"max_id":i32::MAX}));
    }
    writer.start(&files.settings, environment)?;
    let mut dialogs = collect_dialogs(files, input).await?;
    finalize_dialogs_info(&mut dialogs, &files.settings)?;
    writer.write_dialogs_start(&dialogs)?;
    for info in dialogs.chats.iter().chain(&dialogs.left) {
        export_dialog(files, info.clone(), writer, dates).await?;
    }
    writer.write_dialogs_end()?;
    writer.finish()?;
    Ok(())
}
async fn collect_dialogs<C: ExportClient>(
    files: &mut MessageFiles<'_, C>,
    input: &Value,
) -> FetchResult<DialogsInfo> {
    let mut infos = if type_is(input, "InputPeerUser") || type_is(input, "InputPeerSelf") {
        let user = if type_is(input, "InputPeerSelf") {
            json!({"_":"InputUserSelf"})
        } else {
            json!({"_":"InputUser","user_id":input["user_id"],"access_hash":input["access_hash"]})
        };
        let users = files
            .main(json!({"_":"users.GetUsers","id":[user]}))
            .await?;
        parse_dialogs_info_users(input, users.as_array().ok_or(ExportError::InvalidRecord)?)?
    } else if type_is(input, "InputPeerChat") {
        let chats = files
            .main(json!({"_":"messages.GetChats","id":[input["chat_id"]]}))
            .await?;
        parse_dialogs_info_chats(input, array(&chats, "chats"))?
    } else if type_is(input, "InputPeerChannel") {
        let chats=files.main(json!({"_":"channels.GetChannels","id":[{"_":"InputChannel","channel_id":input["channel_id"],"access_hash":input["access_hash"]}]})).await?;
        parse_dialogs_info_chats(input, array(&chats, "chats"))?
    } else {
        return Err(ExportError::InvalidOptions.into());
    };
    let mut result = DialogsInfo::default();
    let mut positions = BTreeMap::new();
    for _ in 0..2 {
        let last = files.splits.len() as i64 - 1;
        let mut migrated = None;
        for chat in &mut infos.chats {
            if matches!(
                chat.r#type,
                DialogType::PrivateSupergroup | DialogType::PublicSupergroup
            ) && migrated.is_none()
            {
                migrated = Some(chat.clone());
                continue;
            }
            if matches!(
                chat.r#type,
                DialogType::PrivateChannel | DialogType::PublicChannel
            ) || chat.is_monoforum
            {
                continue;
            }
            for i in (1..=last).rev() {
                chat.splits.push(i - 1);
                chat.messages_count_per_split.push(0);
            }
        }
        for mut info in infos.chats {
            let good = files.settings.types & settings_from_dialogs_type(info.r#type) != 0;
            if !good
                && !(info.migrated_to_channel_id != 0
                    && files.settings.types & (Settings::PUBLIC_GROUPS | Settings::PRIVATE_GROUPS)
                        != 0)
            {
                continue;
            }
            if info.migrated_to_channel_id != 0 {
                if let Some(index) = positions.get(&peer_from_channel(info.migrated_to_channel_id))
                    && add_migrate_from_slice(
                        &mut result.chats[*index],
                        &info,
                        last,
                        files.splits.len() as i64,
                    )
                {
                    continue;
                }
                if !good {
                    continue;
                }
            }
            let position = *positions.entry(info.peer_id).or_insert(result.chats.len());
            if position == result.chats.len() {
                info.splits.push(last);
                info.messages_count_per_split.push(0);
                result.chats.push(info);
            } else {
                result.chats[position].splits.push(last);
                result.chats[position].messages_count_per_split.push(0);
            }
        }
        let Some(source) = migrated else {
            break;
        };
        let full=files.main(json!({"_":"channels.GetFullChannel","channel":{"_":"InputChannel","channel_id":source.input["channel_id"],"access_hash":source.input["access_hash"]}})).await?;
        let migrated_id = integer(&full["full_chat"], "migrated_from_chat_id");
        infos = if migrated_id != 0 {
            parse_dialogs_info_chats(
                &json!({"_":"InputPeerChat","chat_id":migrated_id}),
                array(&full, "chats"),
            )?
        } else {
            migrated_from_history(files, &source, last).await?
        };
    }
    Ok(result)
}
async fn migrated_from_history<C: ExportClient>(
    files: &MessageFiles<'_, C>,
    source: &DialogInfo,
    split: i64,
) -> FetchResult<DialogsInfo> {
    let response = files
        .split(split, json!({"_":"messages.GetHistory","peer":source.input,"offset_id":1,"offset_date":0,"add_offset":-1,"limit":1,"max_id":0,"min_id":0,"hash":0}))
        .await;
    let response = match response {
        Ok(response) => response,
        Err(FetchError::Rpc { .. }) => return Ok(DialogsInfo::default()),
        Err(error) => return Err(error),
    };
    let id = array(&response, "messages")
        .iter()
        .filter(|message| type_is(message, "MessageService"))
        .filter_map(|message| {
            let action = &message["action"];
            type_is(action, "MessageActionChannelMigrateFrom").then(|| integer(action, "chat_id"))
        })
        .next_back()
        .unwrap_or(0);
    if id <= 0 {
        return Ok(DialogsInfo::default());
    }
    let chats = match files.main(json!({"_":"messages.GetChats","id":[id]})).await {
        Ok(chats) => chats,
        Err(FetchError::Rpc { .. }) => return Ok(DialogsInfo::default()),
        Err(error) => return Err(error),
    };
    let mut result = parse_dialogs_info_chats(
        &json!({"_":"InputPeerChat","chat_id":id}),
        array(&chats, "chats"),
    )?;
    let destination = peer_to_channel(source.peer_id);
    result
        .chats
        .retain(|chat| chat.migrated_to_channel_id == destination);
    Ok(result)
}

fn from_date_offset(settings: &Settings) -> FetchResult<i32> {
    if settings.single_peer_from > 0 {
        i32::try_from(settings.single_peer_from - 1).map_err(|_| ExportError::InvalidOptions.into())
    } else {
        Ok(0)
    }
}

async fn history<C: ExportClient>(
    files: &mut MessageFiles<'_, C>,
    info: &mut DialogInfo,
    local: usize,
    offset: i64,
    offset_date: i32,
    add: i64,
    limit: i64,
) -> FetchResult<Value> {
    let split = info.splits[local];
    let peer = if split >= 0 {
        info.input.clone()
    } else {
        info.migrated_from_input.clone()
    };
    let real_split = if split >= 0 {
        split
    } else {
        files.splits.len() as i64 + split
    };
    loop {
        let request = if info.only_my_messages {
            json!({"_":"messages.Search","peer":peer,"q":"","from_id":if type_is(&info.monoforum_broadcast_input,"InputPeerEmpty"){json!({"_":"InputPeerSelf"})}else{info.monoforum_broadcast_input.clone()},"filter":{"_":"InputMessagesFilterEmpty"},"min_date":0,"max_date":0,"offset_id":offset,"add_offset":add,"limit":limit,"max_id":0,"min_id":0,"hash":0})
        } else {
            json!({"_":"messages.GetHistory","peer":peer,"offset_id":if offset_date != 0 {0} else {offset},"offset_date":offset_date,"add_offset":add,"limit":limit,"max_id":0,"min_id":0,"hash":0})
        };
        match files.split(real_split, request).await {
            Err(e)
                if rpc_message(&e) == "CHANNEL_PRIVATE"
                    && type_is(&peer, "InputPeerChannel")
                    && !info.only_my_messages =>
            {
                info.only_my_messages = true
            }
            other => return other,
        }
    }
}
async fn export_dialog<C: ExportClient>(
    files: &mut MessageFiles<'_, C>,
    mut info: DialogInfo,
    writer: &mut dyn ExportWriter,
    dates: &DateContext,
) -> FetchResult<()> {
    for local in 0..info.splits.len() {
        let result = history(files, &mut info, local, 0, 0, 0, 1).await?;
        if type_is(&result, "MessagesNotModified") {
            return Err(ExportError::InvalidRecord.into());
        }
        let mut count = if type_is(&result, "Messages") {
            array(&result, "messages").len() as i64
        } else {
            integer(&result, "count")
        };
        if !single_message_after(&result, files.settings.single_peer_from) {
            count = 0;
        } else {
            if files.settings.single_peer_from > 0 && !info.only_my_messages {
                let offset_date = from_date_offset(&files.settings)?;
                let from = history(files, &mut info, local, 0, offset_date, 0, 1).await?;
                if type_is(&from, "MessagesSlice") || type_is(&from, "ChannelMessages") {
                    count = from["offset_id_offset"].as_i64().unwrap_or(count);
                }
            }
            if files.settings.single_peer_till != 0 {
                let first = history(files, &mut info, local, 1, 0, -1, 1).await?;
                if !single_message_before(&first, files.settings.single_peer_till) {
                    count = 0;
                }
            }
        }
        info.messages_count_per_split[local] = count;
    }
    writer.write_dialog_start(&info)?;
    let mut context = ParseMediaContext {
        self_peer_id: peer_from_user(files.self_id),
        ..Default::default()
    };
    for local in 0..info.splits.len() {
        let mut next = 1;
        loop {
            let (mut slice, last) = if info.messages_count_per_split[local] == 0 {
                (MessagesSlice::default(), true)
            } else {
                let offset_date = if next == 1 && !info.only_my_messages {
                    from_date_offset(&files.settings)?
                } else {
                    0
                };
                let result = history(files, &mut info, local, next, offset_date, -100, 100).await?;
                if type_is(&result, "MessagesNotModified") {
                    return Err(ExportError::InvalidRecord.into());
                }
                let last = type_is(&result, "Messages");
                (
                    parse_messages_slice(
                        &mut context,
                        array(&result, "messages"),
                        array(&result, "users"),
                        array(&result, "chats"),
                        &info.relative_path,
                        dates,
                    )?,
                    last,
                )
            };
            let split = info.splits[local];
            let real_split = if split >= 0 {
                split
            } else {
                files.splits.len() as i64 + split
            };
            let peer = if split >= 0 {
                info.input.clone()
            } else {
                info.migrated_from_input.clone()
            };
            for message in &mut slice.list {
                if skip_message_by_date(message, &files.settings) {
                    continue;
                }
                if message.rich_message.as_ref().is_some_and(|r| r.part) {
                    let result = files
                        .main(json!({"_":"messages.GetRichMessage","peer":peer,"id":message.id}))
                        .await?;
                    let full = extract_full_rich_message(array(&result, "messages"), message.id)
                        .ok_or(ExportError::InvalidRecord)?;
                    let rich = parse_rich_message(
                        &mut context,
                        &full,
                        &info.relative_path,
                        message.date,
                        dates,
                    )?;
                    if rich.part {
                        return Err(ExportError::InvalidRecord.into());
                    }
                    message.rich_message = Some(rich);
                }
            }
            files.collect_custom_emoji(&slice);
            files
                .resolve_custom_emoji(&mut context, &info.relative_path, dates)
                .await?;
            for message in &mut slice.list {
                if skip_message_by_date(message, &files.settings) {
                    continue;
                }
                files
                    .resolve_message_custom_emoji(&info.relative_path, message, dates)
                    .await?;
                for mut work in message_file_work(message) {
                    let origin = FileOrigin {
                        split: real_split,
                        peer: peer.clone(),
                        message_id: message.id,
                        rich_message: work.rich,
                        ..Default::default()
                    };
                    files
                        .process_file_load(
                            &info.relative_path,
                            &mut work.file,
                            &origin,
                            message,
                            work.media_type,
                            work.controlling_size,
                            dates,
                        )
                        .await?;
                    work.apply(message);
                }
            }
            if slice.list.is_empty() {
                break;
            }
            next = slice
                .list
                .last()
                .ok_or(ExportError::InvalidRecord)?
                .id
                .checked_add(1)
                .ok_or(ExportError::InvalidRecord)?;
            if split < 0 {
                adjust_migrate_message_ids(&mut slice);
            }
            files.messages += slice.list.len();
            writer.write_dialog_slice(&slice)?;
            if last {
                break;
            }
        }
    }
    writer.write_dialog_end()?;
    Ok(())
}
