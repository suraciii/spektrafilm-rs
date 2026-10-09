#!/usr/bin/env python3
"""Independent experimental GUI oracle from pinned Python profiles and manifests.

Generate with the pinned runtime venv (Qt is not required):
  /tmp/spektrafilm-034-venv/bin/python scripts/parity/gui_factory_reference.py generate \
    --repo /path/to/spektrafilm --out-dir scripts/parity/fixtures/gui_28bf883
Compare factory JSON or a native GUI Save state file with the retained oracle:
  python scripts/parity/gui_factory_reference.py check --state path/to/state.json
Arrays count as one leaf. The explicit Rust extension is outside the shared state.
"""
from __future__ import annotations

import argparse
import ast
import dataclasses
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys
from typing import get_args, get_origin, get_type_hints

UPSTREAM = "28bf883e1672e884307edc75852549376e13644e"
FIXTURES = Path(__file__).resolve().parent / "fixtures" / "gui_28bf883"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n")


def source_definition(path, name, namespace):
    """Execute only the named pure definition, excluding Qt imports and setup."""
    tree = ast.parse(path.read_text(), filename=str(path))
    nodes = [node for node in tree.body if (
        isinstance(node, (ast.FunctionDef, ast.ClassDef)) and node.name == name
    ) or (
        isinstance(node, ast.Assign)
        and any(isinstance(target, ast.Name) and target.id == name for target in node.targets)
    )]
    if len(nodes) != 1:
        raise ValueError(f"Expected one definition of {name} in {path}")
    module = ast.Module(body=nodes, type_ignores=[])
    exec(compile(module, str(path), "exec"), namespace)
    return namespace[name]


def annotation_at(root, path):
    annotation = root
    for name in path.split("."):
        annotation = get_type_hints(annotation)[name]
    return annotation


def editor_defaults(path, class_name):
    tree = ast.parse(path.read_text(), filename=str(path))
    cls = next(node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == class_name)
    init = next(node for node in cls.body if isinstance(node, ast.FunctionDef) and node.name == "__init__")
    return {arg.arg: ast.literal_eval(value) for arg, value in zip(init.args.kwonlyargs, init.args.kw_defaults)}


def generate(repo, out_dir):
    repo = repo.resolve()
    head = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"],
                          capture_output=True, text=True, check=True).stdout.strip()
    if head != UPSTREAM:
        raise ValueError(f"Upstream is {head}; expected {UPSTREAM}")
    subprocess.run(["git", "-C", str(repo), "diff", "--quiet", "HEAD", "--", "src"], check=True)
    sys.path.insert(0, str(repo / "src"))
    import spektrafilm_gui.state as state
    import spektrafilm_gui.params_manifest as manifest
    from spektrafilm_gui.options import RawWhiteBalance

    for name, module in tuple(sys.modules.items()):
        if name == "spektrafilm" or name.startswith(("spektrafilm.", "spektrafilm_gui.")):
            source = getattr(module, "__file__", None)
            if source and not Path(source).resolve().is_relative_to(repo / "src"):
                raise ValueError(f"{name} loaded outside pinned upstream: {source}")
    gui = repo / "src" / "spektrafilm_gui"
    # Execute the actual pure serializer body, not a duplicated implementation.
    serializer = source_definition(gui / "persistence.py", "gui_state_to_dict", {
        "GuiState": state.GuiState, "Any": object, "asdict": dataclasses.asdict,
        **{name: getattr(state, name) for name in (
            "input_image_to_dict", "special_to_dict", "simulation_to_dict", "display_to_dict")},
    })
    factory = json.loads(json.dumps(serializer(state.PROJECT_DEFAULT_GUI_STATE)))
    if len(factory) != 21 or len(leaves(factory)) != 187:
        raise ValueError("Pinned factory must contain 21 sections and 187 leaves")
    normalize = source_definition(gui / "widget_primitives.py", "normalize_ui_text", {})
    raw_specs = source_definition(gui / "widget_sections.py", "LOAD_RAW_FIELDS", {
        "ParamSpec": manifest.ParamSpec, "RawWhiteBalance": RawWhiteBalance,
    })
    defaults = {kind: editor_defaults(gui / "widget_editors.py", kind)
                for kind in ("FloatEditor", "IntEditor", "FloatTupleEditor", "IntTupleEditor")}

    def field_spec(spec, annotation):
        result = {field.name: getattr(spec, field.name) for field in dataclasses.fields(spec)
                  if field.name != "enum"}
        result["leaf"] = spec.leaf
        result["declared_label"] = spec.label
        result["label"] = normalize(spec.label or spec.leaf.replace("_", " "))
        result["annotation"] = str(annotation)
        result["enum"] = None if spec.enum is None else [member.value for member in spec.enum]
        if spec.leaf == "development_time":
            result["editor"] = "DevelopmentTimeEditor"
            result["choices"] = "stock-dependent; controller populates from profile"
        elif spec.enum is not None:
            result["editor"] = "ProfileEnumEditor" if spec.leaf in {"film_stock", "print_paper"} else "EnumEditor"
        elif annotation in (bool, str):
            result["editor"] = "BoolEditor" if annotation is bool else "StrEditor"
        else:
            tuple_types = get_args(annotation) if get_origin(annotation) is tuple else None
            integer = annotation is int or (tuple_types and all(item is int for item in tuple_types))
            kind = ("Int" if integer else "Float") + ("TupleEditor" if tuple_types else "Editor")
            base = defaults[kind]
            result["editor"] = kind
            result["effective_numeric"] = {
                "min": base["minimum"] if spec.min is None else spec.min,
                "max": base["maximum"] if spec.max is None else spec.max,
                # QSpinBox/QDoubleSpinBox singleStep defaults to 1; the upstream
                # constructors leave it unchanged when no manifest step is set.
                "step": 1 if spec.step is None else spec.step,
                "decimals": None if integer else (base["decimals"] if spec.decimals is None else spec.decimals),
            }
            if tuple_types:
                result["components"] = len(tuple_types)
        return result

    groups = []
    for group in manifest.ALL_MANIFESTS:
        groups.append({
            "title": group.title, "label": normalize(group.title), "group_path": group.group_path,
            "collapsed_by_default": group.collapsed_by_default,
            "panel_fields": [spec.leaf for spec in (group.panel_fields or group.fields)],
            "subsections": [dataclasses.asdict(section) for section in group.subsections],
            "actions": [dataclasses.asdict(action) for action in group.actions],
            "fields": [field_spec(spec, get_type_hints(group.group_cls)[spec.leaf]) for spec in group.fields],
        })
    root_fields = {}
    for name, root, specs in (
        ("input_image", state.InputImageState, manifest.INPUT_IMAGE_FIELDS),
        ("special", state.SpecialState, manifest.SPECIAL_FIELDS),
        ("simulation", state.SimulationState, manifest.SIMULATION_FIELDS),
        ("display", state.DisplayState, manifest.DISPLAY_PANEL_FIELDS),
        ("load_raw", state.LoadRawState, raw_specs),
    ):
        root_fields[name] = [field_spec(spec, annotation_at(root, spec.path)) for spec in specs]
    panels = {name: [spec.leaf for spec in value] for name, value in vars(manifest).items()
              if name.endswith("_PANEL_FIELDS") and isinstance(value, tuple)}
    metadata = {"upstream_commit": UPSTREAM, "groups": groups, "root_fields": root_fields,
                "panels": panels, "editor_constructor_defaults": defaults,
                "qt_numeric_default_single_step": 1}
    out_dir.mkdir(parents=True, exist_ok=True)
    save_json(out_dir / "factory_state.json", factory)
    save_json(out_dir / "control_metadata.json", metadata)
    source_paths = [gui / name for name in (
        "state.py", "persistence.py", "params_manifest.py", "widget_editors.py",
        "widget_sections.py", "widget_primitives.py", "options.py")]
    data_root = repo / "src" / "spektrafilm" / "data"
    data_hash = hashlib.sha256()
    data_files = sorted(path for path in data_root.rglob("*")
                        if path.is_file() and "__pycache__" not in path.parts)
    for path in data_files:
        data_hash.update(str(path.relative_to(data_root)).encode() + b"\0")
        data_hash.update(path.read_bytes())
        data_hash.update(b"\0")
    provenance = {
        "upstream_url": "https://github.com/andreavolpato/spektrafilm",
        "upstream_commit": UPSTREAM,
        "generation_command": shlex.join([sys.executable, *sys.argv]),
        "python_version": sys.version,
        "serialization": "Actual gui_state_to_dict AST executed without importing Qt persistence module",
        "factory_source": "PROJECT_DEFAULT_GUI_STATE (init_params and digest_params with real profiles/presets)",
        "sections": len(factory), "leaves_arrays_count_as_one": len(leaves(factory)),
        "source_sha256": {str(path.relative_to(repo)): sha(path) for path in source_paths},
        "profile_and_preset_data": {"root": str(data_root.relative_to(repo)),
                                    "files": len(data_files), "tree_sha256": data_hash.hexdigest()},
        "artifact_sha256": {name: sha(out_dir / name) for name in ("factory_state.json", "control_metadata.json")},
    }
    save_json(out_dir / "provenance.json", provenance)
    print(json.dumps({"generated": str(out_dir), **provenance["artifact_sha256"]}, indent=2))


def leaves(value, prefix=""):
    if isinstance(value, dict):
        return {key: leaf for name, child in value.items()
                for key, leaf in leaves(child, f"{prefix}.{name}" if prefix else name).items()}
    return {prefix: value}


def same(actual, expected):
    if isinstance(actual, bool) or isinstance(expected, bool):
        return actual is expected
    if isinstance(actual, list) and isinstance(expected, list):
        return len(actual) == len(expected) and all(same(a, e) for a, e in zip(actual, expected))
    return actual == expected


def check(state_path, fixture_dir, report_path):
    provenance = json.loads((fixture_dir / "provenance.json").read_text())
    if provenance["upstream_commit"] != UPSTREAM:
        raise ValueError("Unexpected retained upstream commit")
    for name, digest in provenance["artifact_sha256"].items():
        if sha(fixture_dir / name) != digest:
            raise ValueError(f"Retained oracle hash mismatch: {name}")
    expected = leaves(json.loads((fixture_dir / "factory_state.json").read_text()))
    if len(expected) != 187:
        raise ValueError("Retained oracle must contain 187 leaves")
    actual_state = json.loads(state_path.read_text())
    actual_state.pop("rust", None)
    actual = leaves(actual_state)
    missing = sorted(expected.keys() - actual.keys())
    extra = sorted(actual.keys() - expected.keys())
    differences = [{"path": path, "expected": expected[path], "actual": actual[path]}
                   for path in sorted(expected.keys() & actual.keys()) if not same(actual[path], expected[path])]
    result = {"upstream_commit": UPSTREAM, "state": str(state_path), "state_sha256": sha(state_path),
              "reference_sha256": sha(fixture_dir / "factory_state.json"),
              "expected_leaves": len(expected), "actual_shared_leaves": len(actual),
              "missing": missing, "extra": extra, "differences": differences,
              "passed": not (missing or extra or differences)}
    if report_path:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        save_json(report_path, result)
    print(json.dumps(result, indent=2))
    return 0 if result["passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generate_parser = commands.add_parser("generate", help="Construct real pinned upstream references")
    generate_parser.add_argument("--repo", required=True, type=Path)
    generate_parser.add_argument("--out-dir", type=Path, default=FIXTURES)
    check_parser = commands.add_parser("check", help="Compare shared factory leaves independently")
    check_parser.add_argument("--state", required=True, type=Path)
    check_parser.add_argument("--fixture-dir", type=Path, default=FIXTURES)
    check_parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if args.command == "generate":
        generate(args.repo, args.out_dir)
        return 0
    return check(args.state, args.fixture_dir, args.report)


if __name__ == "__main__":
    raise SystemExit(main())
