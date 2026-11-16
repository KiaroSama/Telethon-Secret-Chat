# Whole-state storage and scaling

MemoryStorage/FileStorage implement the explicit Storage trait. FileStorage is a
single-writer, versioned whole-state backend: serialize validated state, create a
private uniquely owned staging file, flush it, replace atomically and sync the
parent directory on Unix. Distinct state paths use distinct temporary prefixes;
a competing save/store cannot remove a live write. Unknown or live temporary
owners are preserved. Owner PID/lock and no-follow checks reject invalid owners.

Whole-state commits make counter/key/outbox/mailbox atomicity straightforward but
cost serialization proportional to retained state. Normal files have a finite
128 MiB read/commit ceiling. Bounded/ingress modes add tighter explicit budgets;
this does not make the ordinary backend a scalable message database. Saved
history is independent from protocol retention.

A future alternate backend must preserve the transaction/ownership contracts and
run the paired memory/file contract scenarios with its own fault boundaries.
The previous Windows/Python scratch benchmark is not a Rust performance claim.
Current correctness evidence is the native suite and latest CI checks.
