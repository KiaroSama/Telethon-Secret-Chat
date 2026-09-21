"""The CI evidence gate must refuse false-green matrix scenarios."""

from pathlib import Path
import importlib.util
import xml.etree.ElementTree as ET

import pytest

spec = importlib.util.spec_from_file_location(
    "matrix_gate", Path(__file__).resolve().parents[2] / "tools/check_test_matrix.py"
)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def record(root, leg, *, offline="test_ok", extra_skip=False, failure=False, duplicate=False):
    suite = ET.Element("testsuite")
    for identity in gate.LIVE_SKIPS:
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
