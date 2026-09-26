"""Fail closed on missing matrix evidence, unexpected skips, or case-set drift."""

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
}


def case_set(path: Path):
    root = ET.parse(path).getroot()
    cases, skipped = set(), set()
    for case in root.iter("testcase"):
        identity = f"{case.get('classname', '')}::{case.get('name', '')}"
        if identity in cases:
            raise ValueError("duplicate testcase identity in a matrix record")
        cases.add(identity)
        if case.find("failure") is not None or case.find("error") is not None:
            raise ValueError("a matrix leg reported failing or errored tests")
        if case.find("skipped") is not None:
            skipped.add(identity)
    if skipped != LIVE_SKIPS:
        raise ValueError(
            f"skips differ from the {len(LIVE_SKIPS)} explicitly unexecuted live cases"
        )
    if not cases - skipped:
        raise ValueError("a matrix leg executed no offline tests")
    log.debug("Validated %d cases from one matrix leg", len(cases))
    return cases


def check_matrix(root: Path, expected_legs):
    expected = set(expected_legs)
    if not expected or len(expected) != len(expected_legs):
        raise ValueError("matrix leg names must be nonempty and unique")
    records = list(root.rglob("junit.xml"))
    if {p.parent.name for p in records} != expected or len(records) != len(expected):
        raise ValueError("missing, duplicate, or unexpected matrix artifacts")
    reference = None
    for record in sorted(records):
        cases = case_set(record)
        if reference is not None and cases != reference:
            raise ValueError("matrix legs did not collect identical testcase identities")
        reference = cases
    log.warning("The %d live Telegram cases were intentionally NOT executed", len(LIVE_SKIPS))
    log.info("All %d matrix legs agree on %d testcases", len(records), len(reference))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("legs", nargs="+")
    parser.add_argument("--debug", action="store_true")
    args = parser.parse_args()
    logging.basicConfig(
        level=logging.DEBUG if args.debug else logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
    )
    try:
        check_matrix(args.directory, args.legs)
    except (ValueError, OSError, ET.ParseError) as failure:
        log.error("Matrix evidence rejected (%s)", type(failure).__name__)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
