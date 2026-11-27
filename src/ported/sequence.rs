//! Sequence validation and recovery against offline caller-owned state.

use std::collections::BTreeMap;

use serde_json::{Value, json};

// Reuse the schema codec without linking its separate PyO3 module initializer.
use super::schema_secret_tl as schema;

type Outcome<T> = Result<T, String>;
const WINDOW: i64 = 1000;

fn integer(value: &Value, field: &str) -> Outcome<i64> {
    value
        .get(field)
        .ok_or_else(|| format!("KeyError: {field}"))?
        .as_i64()
        .ok_or_else(|| format!("TypeError: {field} must be an integer"))
}

struct Frame {
    input: i64,
    output: i64,
    layer: i64,
    body: String,
    file: Value,
}

impl Frame {
    fn from_record(record: &Value) -> Outcome<Self> {
        let body = record
            .get("body")
            .ok_or("KeyError: body")?
            .as_str()
            .ok_or("TypeError: body must be hexadecimal text")?;
        let parsed = schema::decode_object(body.to_owned(), "DecryptedMessageLayer".into(), 0)?;
        let encoded_length = body
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .count();
        if parsed["position"]
            .as_u64()
            .and_then(|size| size.checked_mul(2))
            != Some(encoded_length as u64)
        {
            return Err("ValueError: trailing bytes after the wrapper".into());
        }
        let fields = &parsed["object"]["fields"];
        Ok(Self {
            input: integer(fields, "in_seq_no")?,
            output: integer(fields, "out_seq_no")?,
            layer: integer(fields, "layer")?,
            body: parsed["encoded"]
                .as_str()
                .ok_or("ValueError: missing encoded data")?
                .to_owned(),
            file: record["file"].clone(),
        })
    }

    fn fixture(frame: &Value) -> Outcome<Self> {
        if frame.get("body").is_some() {
            return Self::from_record(frame);
        }
        let layer = integer(frame, "layer")?;
        let input = integer(frame, "in_seq_no")?;
        let output = integer(frame, "out_seq_no")?;
        let message =
            json!({"id":0x91cc4674_u32,"fields":{"random_id":1,"ttl":0,"message":"fixture"}});
        let body = schema::encode_object(
            json!({"id":0x1be31789_u32,"fields":{"random_bytes":{"hex":"00".repeat(15)},"layer":layer,"in_seq_no":input,"out_seq_no":output,"message":message}}),
        )?;
        Ok(Self {
            input,
            output,
            layer,
            body,
            file: Value::Null,
        })
    }

    fn index(&self) -> i64 {
        self.output.div_euclid(2)
    }
    fn record(&self) -> Value {
        json!({"seq_no":self.index(),"body":self.body,"file":self.file})
    }
}

pub fn pack_frame(frame: Value) -> Outcome<Value> {
    Ok(Frame::fixture(&frame)?.record())
}

pub fn unpack_frame(record: Value) -> Outcome<Value> {
    let frame = Frame::from_record(&record)?;
    Ok(
        json!({"in_seq_no":frame.input,"out_seq_no":frame.output,"layer":frame.layer,"record":frame.record()}),
    )
}

fn reject(state: &Value, reason: &str) -> String {
    format!(
        "ValueError: MessageRejected: [chat {}] rejected a received message: {reason}",
        state["id"]
    )
}

fn check_order(state: &Value, frame: &Frame) -> Outcome<()> {
    let peer = frame.input.div_euclid(2);
    let reason = if peer < integer(state, "peer_in_seq_no")? {
        "the peer's echo of this side's counter went backwards"
    } else if peer > integer(state, "out_seq_no")? {
        "the peer claims to have seen a message this side has not sent"
    } else if frame.layer < integer(state, "wrapper_layer")? {
        "the peer encoded below the layer it had already used"
    } else {
        return Ok(());
    };
    Err(reject(state, reason))
}

fn check_neighbors(state: &Value, before: &Frame, after: &Frame) -> Outcome<()> {
    if before.input > after.input {
        return Err(reject(
            state,
            "the peer's counter contradicts a buffered neighboring message",
        ));
    }
    if before.layer > after.layer {
        return Err(reject(
            state,
            "the wrapper layer contradicts a buffered neighboring message",
        ));
    }
    Ok(())
}

fn check(state: &Value, frame: &Frame, waiting: &BTreeMap<i64, Value>) -> Outcome<bool> {
    let outbound = state
        .get("is_outbound")
        .and_then(Value::as_bool)
        .ok_or("TypeError: is_outbound must be a boolean")?;
    if frame.input < 0 || frame.output < 0 {
        return Err(reject(state, "sequence numbers must be nonnegative"));
    }
    if frame.input % 2 != i64::from(outbound) || frame.output % 2 != i64::from(!outbound) {
        return Err(reject(
            state,
            "the sequence numbers do not have the parity this side must receive",
        ));
    }
    let actual = frame.index();
    let current = integer(state, "in_seq_no")?;
    if actual < current || waiting.contains_key(&actual) {
        return Ok(false);
    }
    check_order(state, frame)?;
    if let Some((_, before)) = waiting.range(..actual).next_back() {
        check_neighbors(state, &Frame::from_record(before)?, frame)?;
    }
    if let Some((_, after)) = waiting.range(actual..).next() {
        check_neighbors(state, frame, &Frame::from_record(after)?)?;
    }
    if i128::from(actual) - i128::from(current) > i128::from(WINDOW)
        || (actual > current && waiting.len() >= WINDOW as usize)
    {
        return Err(reject(
            state,
            "the incoming gap exceeds the supported recovery window",
        ));
    }
    Ok(true)
}

fn pending_records(frames: Vec<Value>) -> Outcome<BTreeMap<i64, Value>> {
    let mut records = BTreeMap::new();
    for frame in frames {
        let frame = Frame::fixture(&frame)?;
        records
            .entry(frame.index())
            .or_insert_with(|| frame.record());
    }
    Ok(records)
}

pub fn preflight(state: Value, frame: Value, pending: Vec<Value>) -> Outcome<bool> {
    check(&state, &Frame::fixture(&frame)?, &pending_records(pending)?)
}

fn advance(state: &mut Value, frame: &Frame, ready: &mut Vec<Value>) -> Outcome<()> {
    check_order(state, frame)?;
    let next = integer(state, "in_seq_no")?
        .checked_add(1)
        .ok_or("OverflowError: incoming counter out of range")?;
    state["peer_in_seq_no"] = json!(frame.input.div_euclid(2));
    state["wrapper_layer"] = json!(frame.layer);
    state["in_seq_no"] = json!(next);
    ready.push(frame.record());
    Ok(())
}

fn resend_span(current: i64, end: i64, parity: i64) -> Value {
    json!([
        i128::from(current) * 2 + i128::from(parity),
        (i128::from(end) - 1) * 2 + i128::from(parity)
    ])
}

pub fn accept(mut state: Value, frame: Value, pending: Vec<Value>) -> Outcome<Value> {
    let mut records = pending_records(pending)?;
    let mut frame = Frame::fixture(&frame)?;
    let mut resend = Value::Null;
    let mut ready = Vec::new();
    if check(&state, &frame, &records)? {
        frame.file = Value::Null;
        state["layer"] = json!(integer(&state, "layer")?.max(frame.layer));
        let index = frame.index();
        let current = integer(&state, "in_seq_no")?;
        let parity = frame.output.rem_euclid(2);
        if index > current {
            records.insert(index, frame.record());
            if state["gap_requested"] != true {
                state["gap_requested"] = json!(true);
                state["gap_end"] = json!(index);
                resend = resend_span(current, index, parity);
                state["resend_due"] = resend.clone();
            }
        } else {
            advance(&mut state, &frame, &mut ready)?;
            while let Some((&seq, _)) = records.first_key_value() {
                let now = integer(&state, "in_seq_no")?;
                if seq > now {
                    break;
                }
                if let Some(record) = records.remove(&seq)
                    && seq == now
                {
                    advance(&mut state, &Frame::from_record(&record)?, &mut ready)?;
                }
            }
            if let Some((&first, _)) = records.first_key_value() {
                let now = integer(&state, "in_seq_no")?;
                if state["gap_end"].as_i64().is_none_or(|end| now >= end) {
                    state["gap_end"] = json!(first);
                    resend = resend_span(now, first, parity);
                    state["resend_due"] = resend.clone();
                    state["gap_requested"] = json!(true);
                }
            } else {
                state["gap_requested"] = json!(false);
                state["gap_end"] = Value::Null;
                state["resend_due"] = Value::Null;
            }
        }
    }
    Ok(
        json!({"state":state,"ready":ready,"resend":resend,"pending":records.into_values().collect::<Vec<_>>()}),
    )
}

fn retain(retained: Vec<Value>) -> Outcome<BTreeMap<i64, Value>> {
    retained
        .into_iter()
        .map(|item| integer(&item, "seq_no").map(|key| (key, item)))
        .collect()
}

pub fn answer_resend(
    chat_id: i64,
    is_outbound: bool,
    retained: Vec<Value>,
    start: i64,
    end: i64,
) -> Outcome<Vec<Value>> {
    let unsatisfiable = |from| {
        format!(
            "ValueError: ResendUnsatisfiable: [chat {chat_id}] cannot resend messages {start}-{end}: this side retains from {from}. The protocol requires the chat to end rather than answer a resend it cannot satisfy"
        )
    };
    let retained = retain(retained)?;
    let parity = i64::from(is_outbound);
    if start < 0
        || end < start
        || start % 2 != parity
        || end % 2 != parity
        || (end - start) / 2 >= WINDOW
    {
        return Err(unsatisfiable(-1));
    }
    (start..=end)
        .step_by(2)
        .map(|seq| {
            retained.get(&seq).cloned().ok_or_else(|| {
                unsatisfiable(retained.first_key_value().map_or(-1, |(&seq, _)| seq))
            })
        })
        .collect()
}

pub fn forget_acknowledged(
    _chat_id: i64,
    is_outbound: bool,
    retained: Vec<Value>,
    counter: i64,
) -> Outcome<Vec<Value>> {
    let high = (i128::from(counter) - 1) * 2 + i128::from(is_outbound);
    Ok(retain(retained)?
        .into_iter()
        .filter(|(key, _)| counter <= 0 || i128::from(*key) > high)
        .map(|(_, value)| value)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_range_does_not_skip_missing_positions() {
        assert!(
            answer_resend(
                7,
                true,
                vec![json!({"seq_no":1}), json!({"seq_no":5})],
                1,
                5
            )
            .is_err()
        );
        assert_eq!(
            forget_acknowledged(7, true, vec![json!({"seq_no":1}), json!({"seq_no":5})], 1),
            Ok(vec![json!({"seq_no":5})])
        );
    }
    #[test]
    fn packs_and_parses_synthetic_wrapper() -> Outcome<()> {
        let fields = json!({"in_seq_no":1,"out_seq_no":0,"layer":144});
        let record = pack_frame(fields)?;
        assert_eq!(unpack_frame(record)?["in_seq_no"], 1);
        Ok(())
    }
}
