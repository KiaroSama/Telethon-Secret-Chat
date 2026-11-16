# Moving an application to the Rust crate

The Rust API, grammers session representation and native state format are breaking
changes. There is no general Python-store/session importer in this repository.
Do not rename or feed a Python state file to FileStorage, ProtectedFileStorage or
EncryptedIngressStorage. Schema/binding mismatch refuses participation and never
means the old secret chats are safe to recreate under reset counters.

Keep existing state/backups with the exact previous implementation that owns
them. Continue or explicitly close existing chats through that implementation
before choosing a new store for new native chats. A lost transient payload cannot
be reconstructed from its saved key. Protected/bounded/ingress data must retain
its matching keys, policy and custody journal; switching policy is not recovery.
An encrypted ingress journal cannot be replaced by a new live baseline while
protocol obligations remain.

Saved HTML/JSON archives are independent records. They remain viewable with the
retained static viewer assets; they are not protocol state and cannot restore a
chat. Keep encryption material outside published archives and logs.

The separate telegram-mcp Rust rewrite is considered only as a future consumer.
It has not been connected or changed. When its owner requests integration, review
its actual final API/storage/client graph, pin this crate to a reviewed commit,
verify existing account ownership and choose an explicit cutover policy first.
