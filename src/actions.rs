//! Classification of Telegram's closed set of thirteen secret-chat service actions.
use crate::enums::DecryptedMessageAction as Action;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionGroup {
    Conversation,
    Protocol,
    Rekey,
}
impl ActionGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Protocol => "protocol",
            Self::Rekey => "rekey",
        }
    }
}
pub fn group_of(action: &Action) -> ActionGroup {
    match action {
        Action::SetMessageTtl(_)
        | Action::ReadMessages(_)
        | Action::DeleteMessages(_)
        | Action::ScreenshotMessages(_)
        | Action::FlushHistory
        | Action::Typing(_) => ActionGroup::Conversation,
        Action::NotifyLayer(_) | Action::Resend(_) | Action::Noop => ActionGroup::Protocol,
        Action::RequestKey(_)
        | Action::AcceptKey(_)
        | Action::CommitKey(_)
        | Action::AbortKey(_) => ActionGroup::Rekey,
    }
}
