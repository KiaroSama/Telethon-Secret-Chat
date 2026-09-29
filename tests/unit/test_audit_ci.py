"""The CI evidence gate must refuse false-green matrix scenarios."""

from pathlib import Path
import ast
import importlib.util
import xml.etree.ElementTree as ET

import pytest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("matrix_gate", ROOT / "tools/check_test_matrix.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def record(
    root, leg, *, offline="test_ok", extra_skip=False, failure=False, duplicate=False, skips=()
):
    suite = ET.Element("testsuite")
    for identity in gate.LIVE_SKIPS | set(skips):
        module, name = identity.split("::")
        ET.SubElement(ET.SubElement(suite, "testcase", classname=module, name=name), "skipped")
    if offline:
        case = ET.SubElement(suite, "testcase", classname="tests.unit.example", name=offline)
        if extra_skip:
            ET.SubElement(case, "skipped")
        if failure:
            ET.SubElement(case, "failure")
        if duplicate:
            ET.SubElement(suite, "testcase", classname="tests.unit.example", name=offline)
    path = root / leg / "junit.xml"
    path.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(suite).write(path)
    return path


def test_complete_identical_matrix_is_accepted(tmp_path):
    for leg in ("a", "b"):
        record(tmp_path, leg)
    gate.check_matrix(tmp_path, ["a", "b"])


@pytest.mark.parametrize(
    "scenario", ["missing", "extra", "different", "skipped", "failed", "empty", "duplicate"]
)
def test_false_green_evidence_is_rejected(tmp_path, scenario):
    record(tmp_path, "a")
    if scenario == "missing":
        pass
    else:
        record(
            tmp_path,
            "b",
            offline=(
                "different"
                if scenario == "different"
                else None if scenario == "empty" else "test_ok"
            ),
            extra_skip=scenario == "skipped",
            failure=scenario == "failed",
            duplicate=scenario == "duplicate",
        )
    if scenario == "extra":
        record(tmp_path, "unexpected")
    with pytest.raises(ValueError):
        gate.check_matrix(tmp_path, ["a", "b"])


def test_the_summary_counts_the_live_cases_it_skipped(tmp_path, caplog):
    """The warning said "five" after a sixth live case was added."""
    for leg in ("a", "b"):
        record(tmp_path, leg)
    with caplog.at_level("WARNING"):
        gate.check_matrix(tmp_path, ["a", "b"])
    assert f"The {len(gate.LIVE_SKIPS)} live Telegram cases" in caplog.text


def test_the_live_case_list_matches_the_interop_tier():
    """The gate's explicit list must be exactly what tests/interop defines, so a new
    live test fails here in the same change instead of on the next CI run. Parsed,
    not imported: the interop modules need a real client to import cleanly."""
    discovered = set()
    for module in sorted((ROOT / "tests/interop").glob("test_live_*.py")):
        tree = ast.parse(module.read_text(encoding="utf-8"))
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith(
                "test_"
            ):
                discovered.add(f"tests.interop.{module.stem}::{node.name}")
    assert discovered == gate.LIVE_SKIPS


PLATFORM_LEG, PLATFORM_CASE = next(
    (leg, next(iter(cases))) for leg, cases in gate.PLATFORM_SKIPS.items() if cases
)


@pytest.mark.parametrize("leg,accepted", [(PLATFORM_LEG, True), ("cases-other-leg", False)])
def test_a_platform_skip_is_accepted_only_on_its_declared_leg(tmp_path, leg, accepted):
    # The declared leg must skip exactly its whole set; any other leg rejects even one.
    cases = sorted(gate.PLATFORM_SKIPS[PLATFORM_LEG]) if accepted else [PLATFORM_CASE]
    path = record(tmp_path, leg, skips=())
    tree = ET.parse(path)
    for case in cases:
        module, name = case.split("::")
        ET.SubElement(
            ET.SubElement(tree.getroot(), "testcase", classname=module, name=name), "skipped"
        )
    tree.write(path)
    if accepted:
        gate.case_set(path)
    else:
        with pytest.raises(ValueError, match=PLATFORM_CASE):
            gate.case_set(path)


def test_a_rejection_names_what_differs(tmp_path, caplog):
    record(tmp_path, "a")
    record(tmp_path, "b", offline="test_only_on_b")
    with caplog.at_level("ERROR"):
        assert gate.main([str(tmp_path), "a", "b", "--expect-legs", "2"]) == 1
    assert "tests.unit.example::test_only_on_b" in caplog.text


@pytest.mark.parametrize("count,status", [(2, 0), (3, 1)])
def test_the_leg_count_is_asserted(tmp_path, count, status):
    for leg in ("a", "b"):
        record(tmp_path, leg)
    assert gate.main([str(tmp_path), "a", "b", "--expect-legs", str(count)]) == status


@pytest.mark.parametrize("skipped", [True, False])
def test_an_optional_fixture_test_may_skip_or_run(tmp_path, skipped):
    module, name = next(iter(gate.OPTIONAL_SKIPS)).split("::")
    path = record(tmp_path, "a")
    tree = ET.parse(path)
    case = ET.SubElement(tree.getroot(), "testcase", classname=module, name=name)
    if skipped:
        ET.SubElement(case, "skipped")
    tree.write(path)
    gate.case_set(path)
