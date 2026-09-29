"""Fail closed on missing matrix evidence, unexpected skips, or case-set drift.

Usage: ``check_test_matrix.py RECORDS --expect-legs N LEG [LEG ...]``. Every leg is a
directory under RECORDS holding one ``junit.xml``; the leg names come from the
artifacts CI downloaded, and ``--expect-legs`` is what stops a leg that uploaded
nothing from passing unnoticed.

Skips are allowed in exactly three places, and every other skip rejects the matrix:

- ``LIVE_SKIPS``: the interop tier and the pair tier, which need real accounts and never run in
  CI. The list is explicit so the gate still fails closed if discovery breaks, and
  ``tests/unit/test_audit_ci.py`` derives the same list from ``tests/interop`` so a
  new live test that is not listed here fails in the same change.
- ``PLATFORM_SKIPS``: per leg. A POSIX-only check is marked
  ``skipif(os.name == "nt")`` AND listed under the Windows leg here, so that leg's
  junit shows it skipped rather than silently passing, and the same skip on any other
  leg is still a rejection.
- ``OPTIONAL_SKIPS``: allowed on every leg but not required. Only for a test whose
  input is an optional committed fixture that needs the operator to produce (the
  official-client capture): absent, it skips with its reason; committed, it runs.
"""

from __future__ import annotations

import argparse
import logging
from pathlib import Path
import xml.etree.ElementTree as ET

log = logging.getLogger(__name__)
LIVE_SKIPS = {
    "tests.interop.test_live_roundtrip::test_both_ends_agree_on_the_key_fingerprint",
    "tests.interop.test_live_roundtrip::test_a_message_crosses_in_both_directions",
    "tests.interop.test_live_roundtrip::test_closing_here_is_seen_as_closed_here",
    "tests.interop.test_live_media::test_every_media_kind_is_sent_and_opens_on_the_far_side",
    "tests.interop.test_live_media::test_a_file_from_the_far_side_decrypts_and_is_written",
    "tests.interop.test_live_rekey::test_a_rekey_started_here_is_accepted_by_the_official_client",
    # The pair tier: this package on both ends, two real accounts; local-only as well.
    "tests.live_pair.test_pair_chat::test_one_chat_through_text_rekey_files_and_a_restart",
    "tests.live_pair.test_pair_close::test_a_close_reaches_the_other_side_on_its_next_update_sync",
    "tests.live_pair.test_pair_close::test_sending_into_a_chat_the_peer_closed_closes_it_here",
}
PLATFORM_SKIPS = {
    "cases-windows-latest-py3.13": {
        # File modes are meaningless on Windows; the backend's ACL is its own check.
        "tests.unit.test_storage_contract::test_the_file_backend_is_owner_only",
        "tests.unit.test_guards::test_a_symlinked_store_is_refused",
        "tests.unit.test_guards::test_a_directory_fsync_failure_is_a_warning_not_a_rollback",
    },
}
OPTIONAL_SKIPS = {
    "tests.vectors.test_official_client_frames::test_official_client_frames_replay_offline",
}
# Test identities are not secrets, but a broken leg can differ by hundreds of cases;
# the first few are enough to know where to look.
_SHOWN = 20


def _sample(identities):
    return sorted(identities)[:_SHOWN]


def case_set(path: Path, leg: str | None = None):
    leg = leg or path.parent.name
    expected_skips = LIVE_SKIPS | PLATFORM_SKIPS.get(leg, set())
    root = ET.parse(path).getroot()
    cases, skipped = set(), set()
    for case in root.iter("testcase"):
        identity = f"{case.get('classname', '')}::{case.get('name', '')}"
        if identity in cases:
            raise ValueError(f"{leg}: duplicate testcase identity {identity}")
        cases.add(identity)
        if case.find("failure") is not None or case.find("error") is not None:
            raise ValueError(f"{leg}: failing or errored test {identity}")
        if case.find("skipped") is not None:
            skipped.add(identity)
    if not expected_skips <= skipped <= expected_skips | OPTIONAL_SKIPS:
        raise ValueError(
            f"{leg}: skips differ from the {len(expected_skips)} declared ones: "
            f"{_sample((skipped - OPTIONAL_SKIPS) ^ expected_skips)}"
        )
    if not cases - skipped:
        raise ValueError(f"{leg}: executed no offline tests")
    log.debug("Validated %d cases from %s", len(cases), leg)
    return cases


def check_matrix(root: Path, expected_legs, expect_count: int | None = None):
    expected = set(expected_legs)
    if not expected or len(expected) != len(expected_legs):
        raise ValueError("matrix leg names must be nonempty and unique")
    if expect_count is not None and len(expected) != expect_count:
        raise ValueError(f"expected {expect_count} matrix legs, got {sorted(expected)}")
    records = list(root.rglob("junit.xml"))
    found = [p.parent.name for p in records]
    if set(found) != expected or len(records) != len(expected):
        raise ValueError(
            f"missing, duplicate, or unexpected matrix artifacts: expected "
            f"{sorted(expected)}, found {sorted(found)}"
        )
    reference = reference_leg = None
    for record in sorted(records):
        cases = case_set(record)
        if reference is not None and cases != reference:
            raise ValueError(
                f"{record.parent.name} and {reference_leg} collected different testcase "
                f"identities: {_sample(cases ^ reference)}"
            )
        reference, reference_leg = cases, record.parent.name
    log.warning("The %d live Telegram cases were intentionally NOT executed", len(LIVE_SKIPS))
    log.info("All %d matrix legs agree on %d testcases", len(records), len(reference))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("directory", type=Path)
    parser.add_argument("legs", nargs="+")
    parser.add_argument(
        "--expect-legs", type=int, required=True, help="how many legs the matrix defines"
    )
    parser.add_argument("--debug", action="store_true")
    args = parser.parse_args(argv)
    logging.basicConfig(
        level=logging.DEBUG if args.debug else logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
    )
    try:
        check_matrix(args.directory, args.legs, args.expect_legs)
    except (ValueError, OSError, ET.ParseError) as failure:
        log.error("Matrix evidence rejected: %s", failure)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
