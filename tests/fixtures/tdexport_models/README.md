# Native parser and visitor parity vectors

Each JSON file corresponds to one collected legacy pytest scenario. The records
contain data arguments, complete independently captured results, and mutated
arguments from unchanged baseline functions. They were captured in UTC from the
original suite; optional TL fields are recorded from the actual object state so
`None` and an explicitly empty vector remain distinct.

The Rust tests dispatch only to a fixed set of native Rust functions. No runtime
source evaluation, procedure language, case-specific result lookup, or output
substitution occurs. Visitor vectors compare the complete ordered callback event
stream. File-reference and parse-context changes are compared independently of
returned values. The temporary recorder is outside the delivered source tree.
