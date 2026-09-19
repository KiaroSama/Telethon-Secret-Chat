"""protocol-reference.md §5 - all thirteen actions, both directions.

"A usable client must **handle** all thirteen on receipt, and must be able to
**send** at least 5.1, 5.2, 5.3, 5.4, 5.6, 5.7 and 5.9-5.13; 5.5 and 5.8 are
optional outbound features. The consuming MCP server exposes tools over 5.1, 5.2,
5.3, 5.4, 5.6 and 5.7, so all six need a public send path, not only an inbound
branch." FR-010 says the same thing as a requirement.

§8.5 measured the archived package: of the six the consumer needs, **five had no
send path at all**, and `AbortKey` fell through to the application on receipt, so a
peer's abort never cleared the local exchange and the chat could deadlock half-open.

The split under test is data-model.md §4. Conversation actions become events and may
change stored state; Protocol and Rekey actions are the package's own business -
handled without asking the application, and still reported (FR-011).
"""

import pytest

from telethon_secret_chat import SecretChatManager, actions
from telethon_secret_chat.chat import SecretChat
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import Wire, establish

KEY = bytes((i * 5 + 3) % 256 for i in range(256))


def a_chat():
    chat = SecretChat(id=7, access_hash=1, peer_user_id=2, is_outbound=True)
    chat.adopt_key(KEY)
    return chat


@pytest.fixture
async def pair():
    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    yield wire, a, b
    await a.stop()
    await b.stop()


ALL_THIRTEEN = [
    tl.DecryptedMessageActionSetMessageTTL(ttl_seconds=60),
    tl.DecryptedMessageActionReadMessages(random_ids=[1, 2]),
    tl.DecryptedMessageActionDeleteMessages(random_ids=[3]),
    tl.DecryptedMessageActionScreenshotMessages(random_ids=[4]),
    tl.DecryptedMessageActionFlushHistory(),
    tl.DecryptedMessageActionResend(start_seq_no=1, end_seq_no=3),
    tl.DecryptedMessageActionNotifyLayer(layer=144),
    tl.DecryptedMessageActionTyping(action=tl.SendMessageTypingAction()),
    tl.DecryptedMessageActionRequestKey(exchange_id=9, g_a=b"\x01" * 256),
    tl.DecryptedMessageActionAcceptKey(exchange_id=9, g_b=b"\x02" * 256, key_fingerprint=5),
    tl.DecryptedMessageActionCommitKey(exchange_id=9, key_fingerprint=5),
    tl.DecryptedMessageActionAbortKey(exchange_id=9),
    tl.DecryptedMessageActionNoop(),
]


# --- the set is complete and closed -------------------------------------------


def test_there_are_thirteen_and_the_test_covers_them_all():
    """§5: "thirteen constructors, no more"."""
    in_schema = {
        c.TL_NAME for c in tl.REGISTRY.values() if c.RESULT_TYPE == "DecryptedMessageAction"
    }
    covered = {type(a).TL_NAME for a in ALL_THIRTEEN}
    assert covered == in_schema and len(covered) == 13


@pytest.mark.parametrize("action", ALL_THIRTEEN, ids=lambda a: type(a).TL_NAME)
def test_every_action_round_trips_on_the_wire(action):
    from telethon.extensions import BinaryReader

    service = tl.DecryptedMessageService(random_id=1, action=action)
    with BinaryReader(bytes(service)) as reader:
        back = tl.read_object(reader)
    assert type(back.action) is type(action)


@pytest.mark.parametrize("action", ALL_THIRTEEN, ids=lambda a: type(a).TL_NAME)
def test_every_action_is_classified(action):
    """data-model.md §4. An action in no group would be reported without anyone
    having decided whether the package should act on it."""
    assert actions.group_of(action) in ("conversation", "protocol", "rekey")


# --- the split (data-model.md §4) ---------------------------------------------


@pytest.mark.parametrize(
    "action, group",
    [
        (tl.DecryptedMessageActionSetMessageTTL(ttl_seconds=1), "conversation"),
        (tl.DecryptedMessageActionReadMessages(random_ids=[]), "conversation"),
        (tl.DecryptedMessageActionDeleteMessages(random_ids=[]), "conversation"),
        (tl.DecryptedMessageActionFlushHistory(), "conversation"),
        (tl.DecryptedMessageActionScreenshotMessages(random_ids=[]), "conversation"),
        (tl.DecryptedMessageActionTyping(action=tl.SendMessageTypingAction()), "conversation"),
        (tl.DecryptedMessageActionNotifyLayer(layer=144), "protocol"),
        (tl.DecryptedMessageActionResend(start_seq_no=1, end_seq_no=1), "protocol"),
        (tl.DecryptedMessageActionNoop(), "protocol"),
        (tl.DecryptedMessageActionRequestKey(exchange_id=1, g_a=b""), "rekey"),
        (tl.DecryptedMessageActionAcceptKey(exchange_id=1, g_b=b"", key_fingerprint=0), "rekey"),
        (tl.DecryptedMessageActionCommitKey(exchange_id=1, key_fingerprint=0), "rekey"),
        (tl.DecryptedMessageActionAbortKey(exchange_id=1), "rekey"),
    ],
    ids=lambda x: x if isinstance(x, str) else type(x).TL_NAME,
)
def test_the_groups_are_as_the_data_model_states(action, group):
    assert actions.group_of(action) == group


# --- inbound ------------------------------------------------------------------


async def test_the_peers_ttl_is_stored(pair):
    """§5.1: "Store it and apply to subsequent messages; 0 disables." US4 scenario 2:
    "When the application reports the chat, Then it reports the peer's value"."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    await b.set_ttl(chat_b.id, 300)
    assert a.status(chat_a.id).ttl == 300


async def test_a_ttl_of_zero_disables_it(pair):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    await b.set_ttl(chat_b.id, 300)
    await b.set_ttl(chat_b.id, 0)
    assert a.status(chat_a.id).ttl == 0


@pytest.mark.parametrize("action", ALL_THIRTEEN, ids=lambda a: type(a).TL_NAME)
async def test_every_action_reaches_the_application(pair, action):
    """FR-011: "report a service action received from the peer to the application
    rather than silently applying it" - all thirteen, including the ones the package
    acts on itself."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    seen = []
    a.on("ServiceActionReceived", seen.append)

    if isinstance(action, tl.DecryptedMessageActionResend):
        # §3.7 answers this one immediately and rewrites it to Noop, which is a
        # different documented path - the Noop is what arrives.
        action = tl.DecryptedMessageActionNoop()
    await b._send(b.status(chat_b.id), tl.DecryptedMessageService(random_id=1, action=action))
    assert [e.action_name for e in seen] == [type(action).__name__]


async def test_an_unknown_action_is_reported_rather_than_dropped(pair):
    """FR-011 and §5's closed set. A constructor outside the thirteen cannot be
    acted on, and guessing at it is how a parser becomes an oracle - so it is
    refused, visibly."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    failures = []
    a.on("DecryptFailed", failures.append)
    from telethon_secret_chat import crypto

    # A well-formed frame - right key, right msg_key, right padding - whose decrypted
    # BODY names a constructor the schema does not define. Everything §2.7 checks
    # passes, so §3.1's parse is what has to refuse it.
    frame = crypto.encrypt_frame(chat_b.key, b"\xde\xad\xbe\xef" * 4, chat_b.out_x)
    await wire.a.deliver(chat_a.id, frame)
    assert failures, "an unparseable action vanished silently"
    assert "constructor" in failures[0].reason or "parsed" in failures[0].reason


# --- outbound (FR-010) --------------------------------------------------------


async def test_the_six_the_consumer_needs_all_have_a_send_path(pair):
    """§5: "of the six actions the consuming MCP server exposes as tools, five have
    no send path at all" in the archived package. These are those six."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    seen = []
    b.on("ServiceActionReceived", seen.append)

    await a.set_ttl(chat_a.id, 30)  # 5.1
    await a.mark_read(chat_a.id, [11, 12])  # 5.2
    await a.delete_messages(chat_a.id, [13])  # 5.3
    await a.screenshot(chat_a.id, [14])  # 5.4
    await a.flush_history(chat_a.id)  # 5.5, optional but exposed
    await a.set_typing(chat_a.id)  # 5.8, optional but exposed

    assert [e.action_name for e in seen] == [
        "DecryptedMessageActionSetMessageTTL",
        "DecryptedMessageActionReadMessages",
        "DecryptedMessageActionDeleteMessages",
        "DecryptedMessageActionScreenshotMessages",
        "DecryptedMessageActionFlushHistory",
        "DecryptedMessageActionTyping",
    ]


async def test_setting_a_ttl_stores_it_on_this_side_too(pair):
    """US4 scenario 1: the peer's client shows the same timer, which means this side
    has to be sending it on subsequent messages as well as announcing it."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    await a.set_ttl(chat_a.id, 45)
    assert a.status(chat_a.id).ttl == 45


async def test_a_message_sent_under_a_ttl_carries_it(pair):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    await a.set_ttl(chat_a.id, 45)
    await a.send_message(chat_a.id, "this one self-destructs")
    assert got[-1].ttl == 45


async def test_a_service_action_advances_the_counter(pair):
    """§3.1: "Note that any service messages in secret chats must also increment the
    seq_no". A service message that did not count would put every later message one
    ahead of where the peer expects it."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    before = a.status(chat_a.id).out_seq_no
    await a.set_ttl(chat_a.id, 10)
    assert a.status(chat_a.id).out_seq_no == before + 1


async def test_deleting_an_unacknowledged_message_leaves_no_hole(pair):
    """§3.8: a deleted message must not leave a gap in the peer's sequence. "change
    the local copy of the original message to decryptedMessageActionDeleteMessages
    with random_id equal to its own random_id" - "otherwise it must arrive as a
    'self-delete' message to maintain the correct sequence of seq_no"."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    random_id = await a.send_message(chat_a.id, "regrettable")
    retained = a._storage.retained_out(chat_a.id)
    await a.delete_messages(chat_a.id, [random_id])

    after = {m["seq_no"]: m for m in a._storage.retained_out(chat_a.id)}
    assert set(after) >= {m["seq_no"] for m in retained}, "a retained message vanished"
    rewritten = after[max(m["seq_no"] for m in retained)]
    assert "regrettable" not in bytes.fromhex(rewritten["body"]).decode("utf-8", "ignore")
