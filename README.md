# fuSHion

Turn a picture into a buildable model made of LEGO®-compatible bricks.

You give fuSHion a photo or drawing. Claude looks at it and describes the
subject as simple shapes. A deterministic engine turns those shapes into real
parts, checks that the model holds together, and renders it from six sides so
Claude can compare it with the picture and refine it. You can keep editing by
chatting ("make the ears bigger") or by clicking bricks. When it's done you get
an LDraw file with build steps, a parts list and a BrickLink wanted list. Where
no real part fits, fuSHion can design a custom 3D-printable brick.

> LEGO® is a trademark of the LEGO Group, which does not sponsor, authorise or
> endorse this project. fuSHion works with LEGO-compatible parts data from
> the LDraw and Rebrickable communities.

## How it works

```
picture ─▶ Claude writes a shape spec ─▶ compile to coloured voxels
        ─▶ pack into real parts (bricks, plates, slopes) ─▶ structure checks
        ─▶ six-view render ─▶ Claude reviews and edits the spec (≤ 5 rounds)
```

- **No image-to-3D model.** Claude infers the 3D form from the picture and
  writes it in a small shape language: ellipsoids, boxes, cylinders, cones,
  capsules, extrusions, groups, mirroring and paint (see
  [`crates/sf-spec/GUIDE.md`](crates/sf-spec/GUIDE.md)). Claude never places
  individual bricks; research on LLM brick design (BrickGPT, BrickBench,
  BrickWright) shows that placing bricks directly fails while writing a build
  program against a validating engine works.
- **Three modes:** a free-standing sculpture, a relief (a picture built lying
  flat, studs up; it can sample the picture directly and drop a plain
  background), or a scene of grouped objects on a baseplate.
- **Part budget.** You choose the maximum number of parts; the engine picks the
  largest size that fits.
- **The packer** builds in three-plate courses with bricks, fills the rest with
  plates, fits 45° slopes (normal and inverted) where the surface falls away,
  alternates the long axis course by course and avoids seams that line up, so
  layers interlock. A repair pass re-packs the area around floating parts and
  single-stud joints. Small features that stick out sideways get an anchor so
  a part can hold them.
- **Structure checks:** overlaps, stud connections, floating groups,
  single-stud joints (Tarjan bridges) and balance (centre of mass inside the
  footprint).
- **Real parts only.** Part and colour availability comes from Rebrickable's
  data: never-produced pairs are not used, and rare pairs (in three sets or
  fewer) are avoided.

## Install

Needs Rust 1.95 or newer.

```sh
git clone https://github.com/shuff57/fuSHion
cd fuSHion
cargo install --path crates/fushion
fushion sync          # downloads the LDraw parts library (~150 MB) and the Rebrickable catalog
```

`sync` uses Rebrickable's public data dumps. With a free
[Rebrickable API key](https://rebrickable.com/api/) in
`~/.config/fushion/rebrickable.key` (or `REBRICKABLE_API_KEY`) it also maps
parts and colours to BrickLink ids for the wanted list. Without the LDraw
library fuSHion still works, with simplified part meshes.

## Use it

### Desktop app

```sh
fushion
```

Open a picture, set the part budget, choose how to reach Claude, and press
**Design**. Edit by chatting or directly in the 3D view: click a part to
select it, <kbd>Delete</kbd> to remove, pick a colour to recolour, arrow keys
and <kbd>PgUp</kbd>/<kbd>PgDn</kbd> to move, <kbd>R</kbd> to rotate, **Add
part** to place new parts, <kbd>Ctrl</kbd>+<kbd>Z</kbd> to undo. The step
slider plays the build steps. **Export** writes the files.

### Two ways to use Claude

| | What you need | How |
|---|---|---|
| **Claude Code** (your Claude subscription) | [Claude Code](https://claude.com/claude-code) installed and logged in | Pick "Claude Code" in the app, or `fushion design photo.jpg --backend claude-code`. fuSHion runs `claude -p` with its own MCP server attached. |
| **Claude API** (pay per use) | An Anthropic API key in `ANTHROPIC_API_KEY`, or pasted into the app's settings (saved to `~/.config/fushion/anthropic.key`, mode 600) | Pick "Claude API" in the app, or `fushion design photo.jpg`. Uses `claude-opus-5-5`. |

### Use it from Claude Code directly

fuSHion is also an MCP server, so you can design models in any Claude Code
session:

```sh
claude mcp add fushion -- fushion mcp --session ~/fushion-session
```

Then ask Claude: *"Use fuSHion to build ~/Pictures/fox.jpg as a brick
model, about 400 parts."* Tools: `get_guide`, `set_source_image`,
`view_source_image`, `set_budget`, `build_model`, `get_state`,
`export_model`, `make_custom_part`, `make_custom_part_from_commands`. Open the same session folder in the desktop app
(`fushion gui --session ~/fushion-session`) to watch builds appear live.

### Command line

```sh
fushion design photo.jpg --budget 400 [--backend api|claude-code] [--prompt "make it a relief"]
fushion build spec.json [--image photo.jpg] [--budget 400]   # no Claude: build a spec you wrote
fushion export --session <dir> <out-dir>
fushion custom '{"kind":"slope45","w":2,"d":3,"plates":3}' --out parts   # a printable part
fushion guide                                                # print the shape language
```

Every session is a folder (by default under `~/.local/share/fushion/sessions`)
holding `source.png`, `spec.json`, `model.json`, `report.txt`, `render.png`
and the spec history.

## Output

- `<title>.ldr`: LDraw model with build steps. Opens in BrickLink Studio,
  LeoCAD and LDCad.
- `<title>-parts.csv`: parts list with LDraw, BrickLink and colour ids.
- `<title>-bricklink.xml`: BrickLink wanted list (upload under *Wanted →
  Upload*).
- Custom parts: STL and 3MF files, millimetres.

## Custom bricks

When a model needs a part that doesn't exist (a colour never produced, a rare
part, a cluster of tiny parts that would be better as one piece), fuSHion
generates it with [SolidCraft](https://github.com/tangency-labs/solidcraft)'s
parametric CAD engine, linked in-process. Export writes STL and 3MF files for
every part whose colour was never produced or is rare, plus
`custom/custom-parts.csv`. Templates cover boxes, 45° slopes, curved slopes and
any footprint of studs; through MCP, Claude can also write freeform parts as
SolidCraft command batches, which are checked for volume, size and a closed
mesh before anything is written. Every part's volume is verified against the
expected value at each modelling stage.

SolidCraft adds roughly 15 minutes to the first build. To skip it:
`cargo install --path crates/fushion --no-default-features`. The default connector uses the
standard FDM-tuned dimensions also used by Slant 3D's free brick tool (8 mm
pitch, Ø4.8 mm studs, 0.1 mm side clearance), with a centre rib through every
second tube of 2-wide bricks and a 0.2 mm bottom chamfer against elephant's foot.
An experimental Slant v2 stud, modelled on the design Slant 3D showed in *We Made
a Better LEGO (For 3D Printing)*, is included for test printing: neighbouring
studs along each long edge fuse into a peanut whose lobes each carry a leaf-spring
fin, cut free by a 0.5 mm hooked slot and a pocket under it, that presses 0.15 mm
on the receiving brick's wall. Its outline is reshaped to fit standard bricks: each
lobe is the Ø4.8 mm stud circle on its inner side (tangent to the tubes, like a
normal stud), the 1.39 mm waist clears the tubes beside it by 0.05 mm, and a fin
never touches a tube. Because the two studs are fused, a brick pushed on must not
end, or run its side wall, between the two studs of a pair: its wall would land on
the waist. Print the files yourself or upload them to a print
service.

## Workspace

| Crate | Role |
|---|---|
| `sf-core` | Grid units, colours (CIEDE2000), part catalog, placements, LDraw transforms |
| `sf-spec` | The shape language and its compiler to coloured voxels |
| `sf-engine` | Packing, slopes, repair, structure checks, build steps, reports |
| `sf-ldraw` | LDraw library install, parser and meshes |
| `sf-render` | Headless CPU renderer for review sheets |
| `sf-catalog` | Rebrickable availability index and BrickLink mapping |
| `sf-export` | LDraw, CSV and BrickLink writers (and an LDraw reader) |
| `sf-custom` | Custom printable bricks via SolidCraft |
| `sf-agent` | Sessions, the Claude API loop, the Claude Code backend |
| `fushion` | Desktop app (egui + wgpu), CLI and MCP server |

## Credits and licences

fuSHion is MIT licensed; see `NOTICE` for SolidCraft's notices. It downloads, but does not redistribute, the
[LDraw parts library](https://library.ldraw.org) (CC BY 4.0 and earlier
licences; see the library's `CAreadme.txt`) and
[Rebrickable](https://rebrickable.com) catalog data. SolidCraft is MIT OR
Apache-2.0; its `solidcore` kernel is Apache-2.0. Ideas for packing, structure
checks and build ordering came from reading brick-it, brickify,
everything2lego, image-to-lego and BrickGPT (all MIT); no code was copied.
The stability approach follows BrickGPT/StableLego (Pun et al., 2025).
