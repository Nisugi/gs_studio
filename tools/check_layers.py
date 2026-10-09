"""The crate layers' check (docs/crate-split-plan.md): what each runtime
crate's normal dependency tree may not hold.

    python tools/check_layers.py            report what breaks the rules
    python tools/check_layers.py --enforce  and fail (exit 1) when anything does

A crate not in the workspace yet is skipped. Until the split's S7 the
check only reported; from S7 it holds: run it with --enforce after any
change to a manifest (and in CI, once there is one).
"""
import json
import subprocess
import sys

AUTHORING = ["rig", "rig_bake", "gs_puppet_author", "gs_calibrators", "studio"]
# what each crate's tree must not hold
RULES = {
    # Hydra's dependency: the doll, its base and nothing of the field or of
    # authoring. (Not toml_edit: toml 0.8 parses through it. The field's
    # sidecar writer that used it is kept out with the field itself.)
    "injury_doll": AUTHORING + ["include_dir", "regex", "serde_json", "gs_field", "gs_field_egui", "creature_field", "creature_field_egui", "eframe"],
    # Hydra's calls: what Hydra's doll pulls in, through injury_doll
    "hydra_contract": AUTHORING + ["include_dir", "gs_field", "gs_field_egui"],
    "creature_field_egui": AUTHORING + ["injury_doll"],
    "creature_field": AUTHORING + ["injury_doll", "egui", "wgpu"],
    "gs_render": AUTHORING + ["include_dir", "gs_field", "creature_field"],
    "gs_base": AUTHORING + ["egui", "wgpu", "egui-wgpu", "include_dir"],
    "gs_puppet": ["rig", "rig_bake", "gs_puppet_author", "egui", "wgpu", "egui-wgpu", "include_dir"],
}


def members():
    meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], capture_output=True, text=True, check=True).stdout)
    return {p["name"] for p in meta["packages"]}


def tree(crate):
    out = subprocess.run(["cargo", "tree", "-e", "normal", "-p", crate, "--prefix", "none", "--format", "{p}"], capture_output=True, text=True, check=True).stdout
    return {line.split()[0] for line in out.splitlines() if line.strip()}


def main():
    enforce = "--enforce" in sys.argv[1:]
    have = members()
    broken = 0
    for crate, forbidden in RULES.items():
        if crate not in have:
            print(f"{crate}: not in the workspace yet")
            continue
        found = sorted(tree(crate) & set(forbidden))
        if found:
            broken += 1
            print(f"{crate}: holds {', '.join(found)}")
        else:
            print(f"{crate}: clean")
    if broken and enforce:
        sys.exit(1)


if __name__ == "__main__":
    main()
