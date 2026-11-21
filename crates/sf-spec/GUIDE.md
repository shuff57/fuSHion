# fuSHion shape spec (version 1)

You describe the subject as simple shapes. The engine turns them into real
bricks, checks the build holds together, and renders it so you can compare it
with the source image. You never place individual bricks.

## Frame and units
- `+y` is up. The ground is the lowest point of the model.
- The subject's **front faces `-z`**. The viewer stands at `-z` looking toward
  `+z`, so `+x` is the viewer's right (the subject's left).
- Use any units you like, with real proportions: one unit up is as long as one
  unit across. A good habit is to make the subject about 20 units tall.
- The engine scales the model so its longest side is `resolution` studs, or
  picks the largest size that fits `budget` parts. One stud is 8 mm; bricks are
  9.6 mm tall, so the grid is finer vertically (plates are 3.2 mm).

## Top level
```json
{
  "version": 1,
  "title": "Red fox",
  "mode": "sculpture",          // sculpture | relief | scene
  "budget": 400,                // max parts (optional)
  "resolution": 24,             // longest side in studs (optional; overrides budget)
  "default_color": "Orange",    // for shapes without a colour
  "shapes": [ ... ],            // sculpture and scene
  "relief": { ... },            // relief only
  "base": { "color": "Dark Bluish Gray", "margin": 1, "plates": 1 },  // optional baseplate
  "notes": "free text you want to keep between rounds"
}
```

## Shapes
Every shape has `"shape"`, geometry fields, and optional `id`, `color`, `op`,
`rotate` and `mirror_x`. Shapes apply **in order**; later shapes win.

| shape | fields | notes |
|---|---|---|
| `box` | `center`, `size` | `size` is full width/height/depth |
| `ellipsoid` | `center`, `size` | spheres, heads, bodies |
| `cylinder` | `center`, `size`, `axis` (`x`/`y`/`z`, default `y`) | `size` along the axis is the length |
| `cone` | `center`, `size`, `axis`, `top_scale` (0–1) | tip points toward `+axis`; `top_scale` > 0 makes a frustum. Point it down with `"rotate": [180,0,0]` |
| `capsule` | `from`, `to`, `radius` | rods: legs, arms, tails, necks |
| `extrude` | `plane`, `points`, `depth` | polygon outline: `xz` footprint extruded along y; `xy` front silhouette extruded along z; `zy` side profile extruded along x |
| `group` | `at`, `rotate_y`, `scale`, `children` | move several shapes as one (scenes, repeated items) |

- `op`: `add` (default), `subtract` (carve away), or `paint` (recolour what is
  already there without adding volume; use for eyes, patches, stripes, logos).
  Paint is slightly enlarged so small details survive.
- `rotate`: `[rx, ry, rz]` degrees about the shape's centre (box, ellipsoid,
  cylinder, cone).
- `mirror_x: true` adds a copy mirrored across `x = 0`. Model symmetric
  subjects centred on `x = 0` and mirror ears, eyes, legs and arms.

## Colours
Use these names (or `#RRGGBB`, which snaps to the nearest one):
Black, White, Light Bluish Gray, Dark Bluish Gray, Very Light Bluish Gray,
Red, Dark Red, Orange, Bright Light Orange, Yellow, Bright Light Yellow,
Tan, Dark Tan, Light Nougat, Nougat, Medium Nougat, Reddish Brown, Dark Brown,
Brown, Lime, Medium Lime, Bright Green, Green, Dark Green, Olive Green,
Sand Green, Dark Turquoise, Medium Azure, Dark Azure, Medium Blue, Blue,
Dark Blue, Bright Light Blue, Sand Blue, Lavender, Medium Lavender,
Dark Purple, Magenta, Dark Pink, Bright Pink, Sand Red.

## Relief mode
A picture built lying flat, studs up. The top row of the picture is the far
edge, so the top view reads like the image.

Either draw it yourself:
```json
"relief": {
  "rows": ["..yy..", ".yyyy.", "yykkyy"],
  "legend": {"y": "Yellow", "k": "Black"},
  "heights": {"rows": {"rows": ["002200", "023320", "000000"]}},
  "base_plates": 1
}
```
or sample the source image directly and only choose the size and colours:
```json
"relief": {"source": "image", "width": 48, "max_colors": 8, "drop_background": true,
           "heights": {"luminance": {"max_plates": 3}}}
```
`drop_background` removes the region touching the picture's border that has
the border's colour (plain studio or sky backgrounds). Reliefs are reviewed
from the top and iso views only.
`heights` is `"flat"` (default), `{"rows": {"rows": [...]}}` with one character
per cell (`0`–`9`, then `a`–`z` = 10–35 extra plates), or
`{"luminance": {"max_plates": N}}`.

## Scene mode
Use `group`s for each object (a house, a tree, a car) and place them with
`at`. Add a `base` so everything stands on one baseplate. Keep objects
touching the ground or each other; floating pieces are reported as errors.

## Designing details
Replicate the picture as closely as you can. Bricks make the masses; every feature that
bricks can't capture (a flower, a petal, a leaf or fern frond, a stem, an eye, a nose, a
beak, a horn, an ear tip) becomes a printed **detail** that you design:
1. `design_detail` with a template (`list_details` shows them all with their parameters)
   or your own SolidCraft commands. Name it for what it is (`red_rose`, `owl_eye`).
2. Look at the picture it returns (iso, front, right, top). Check the shape, size and
   proportions against the source picture; change the parameters and design again until
   it reads right. A design is saved under its name; designing the same name replaces it.
3. Place it, as many times as the picture shows, with the spec's `details`. One design
   can be used many times in different colours.

**Flowers follow LEGO's Botanical bouquets** (10280 Flower Bouquet, 10313 Wildflower
Bouquet): each head is a hub with petals arranged around it, often in two layers offset
by half a petal, with a separate centre in a contrasting colour. Pick the species
template that matches the picture: `rose` (two layers of large cupped petals around a
spiral of small inner petals), `daisy` (one ring of rounded petals around a round
yellow centre), `aster` and `gerbera` (many narrow petals in concentric rings), `poppy`
(four large cupped petals, flat dark centre), `cornflower` (a ring of trumpet florets),
`spike` for snapdragon, lupin, lavender and larkspur (rings of florets stacked up a stem,
smaller toward the tip), `umbel` for cow parsley, and `fern` / `leaf` for foliage. Count
the petals in the picture and match them; match each flower's colour and its size
relative to the rest of the model (a LEGO rose head is about 32 mm, a daisy about 16 mm;
1 stud = 8 mm). A head can stand on its own `stem_height`; tall stems are thin, so keep
them short on a model that will be handled.

## Details (printed parts)
Bricks can't make a petal, an eye, a beak or a leaf. For features like these, design a
printed **detail** part first (see "Custom printable parts" below), then place it with
`details`:
```json
"details": [
  {"design": "daisy", "at": [0, 14, -2], "turn": 0, "color": "White",
   "colors": {"centre": "Yellow", "stem": "Green"}},
  {"design": "leaf", "at": [1.5, 9, -1], "turn": 1, "color": "Green"}
]
```
A detail is a **kit** of separately printed parts, like LEGO's botanical flowers: petals
clipped onto a hub, a centre pressed in, a stem, leaves. Each part has a colour **role**
(`main`, `centre`, `stem`, `leaf`, `pupil`…; the design lists its own). `color` sets the
`main` role; `colors` sets any others, and roles left out keep the design's defaults.
So one design serves a red rose and a yellow rose. An unknown role is an error that lists
the design's roles.
- `design`: the name of a design you made. An unknown name is an error that lists the
  designs that exist.
- `at`: where the centre of the detail's base should sit, in the same units as the
  shapes (relief: `[column, plates, row]` from the picture's top left). The engine puts
  it on the highest surface of the model under its base at or below that height.
- `turn`: quarter turns about the vertical axis (0–3). `color`: the `main` role,
  defaulting to `default_color`. `colors`: other roles by name.
- A detail must sit on the model's top surface, not overlap it or another detail. Only
  what its parts really fill counts: a leaf's blade can rise over a pot's rim as long as
  it clears it. Its base must cover at
  least two studs in a row, since a fused stud can't stand alone. The report warns when
  a detail is skipped, or has no stud under its base and would fall off.

## Building for 3D printing
When the model is built for printing, every brick is designed for its place: same-colour
areas merge into bricks of any shape up to 8 × 8 studs. A feature only one stud wide in
both directions (a 1 × 1 pole, a single-stud nub) can't be held, because its stud would
stand alone. Make thin parts at least two studs long along one direction, or use a
detail.

Surfaces are smooth in print mode: where a brick's outside is exposed, it is cut to the
true shape you described (a dome is a dome, a back is rounded), not stair-stepped. Studs
appear only where another part sits; exposed tops are smooth. So describe the real
curves with ellipsoids, cones and capsules instead of approximating them with boxes.
Anything that overhangs more than 45° (the underside of a ball, a jutting chin) keeps
its true shape, but its bricks are flagged as needing supports when printed. Where you
have a choice, let shapes sit on their widest part (a dome rather than a ball, a head
resting on the body rather than hanging off it).

## How to work
1. Look at the image. Pick the mode. Note the 3–6 features that make the
   subject recognisable (silhouette, proportions, colour regions, eyes).
2. Block the big masses first (body, head), then limbs, then paint details.
   Design printed details for features bricks can't make (flowers, petals, leaves,
   eyes, beaks) and place them with `details`.
3. Build, then read the report and the rendered views next to the image.
   Check silhouette, proportions, key features and colour regions, in that
   order. Fix the spec, not individual bricks.
4. Stop when it reads clearly, or after four rounds.

## Report terms
- **floating**: parts not connected to the ground. Make the shapes overlap
  or touch, or add a support.
- **weak joint**: a group of parts held by a single stud. Thicken the neck,
  overlap more, or widen the contact.
- **off balance**: the centre of mass is outside the footprint. Widen the
  base, add a stand or a baseplate.
- **no effect**: a shape was too small to show at this size. Make it bigger
  or raise the resolution.

## Custom printable parts
When the report lists a custom part candidate (a colour never produced, a
rare part) or a shape no catalog part can make, design a printable part:
- `make_custom_part` with a template: `{"kind":"box","w":2,"d":4,"plates":3}`,
  `{"kind":"slope45","w":2,"d":3,"plates":3}`, `{"kind":"curve","w":1,"d":3,"plates":3}`
  or `{"kind":"footprint","cells":[[0,0],[1,0],[1,1]],"plates":3}`. Sizes are
  studs and plates; slopes fall toward -Y. Every part has fused studs (Slant
  3D's v2 stud reshaped to fit standard bricks): each row's studs are joined in
  pairs, and an odd run of 3 or more gets one triple as near its middle as it
  can go (3 = T, 5 = P+T, 7 = P+T+P, 9 = P+P+T+P), with a leaf-spring fin on
  each end lobe that presses the wall of the brick on top by 0.15 mm (it is
  freed by a hooked slot and a pocket in the top that follows the fin 0.25 mm
  out, showing as a thin even gap around it). A stud
  can't stand alone in its row, so a 1×1 (or a 1-wide slope, or a footprint
  that leaves a lone stud whichever way its rows run) is refused: use a real
  part. Bricks pushed onto a custom part must start and end on a pair or triple,
  never between its studs (their wall would land on a waist). 1-wide parts have
  no pins: the fins of the brick below grip their walls. The profile is
  `StudProfile::fused`: 1.4 mm top, centre ribs, 0.2 mm bottom chamfer, 0.4 mm
  rounded vertical edges. `compliant_stud` is accepted and ignored. Not yet
  settable through `make_custom_part`: `wall_ribs` (off by default; on fused
  bricks only on the end walls, two 0.6 mm ribs per stud centred 1.7 mm either
  side of it, 0.80 mm proud, pressing a Ø4.8 stud from below by 0.05 mm, so the
  fins of a brick below turned 90° meet them by at most 0.06 mm) and
  `edge_radius`.
- `make_custom_part_from_commands` for anything else, written as a SolidCraft
  command batch in millimetres (8 mm pitch, 9.6 mm brick, Ø4.8 × 1.8 mm studs,
  Ø6.51/4.8 mm tubes, 1.2 mm walls, 0.1 mm side clearance) with an `expect`
  block for the volume and bounding box.

Export already writes printable files (fused studs) for never-produced and rare
colours; a part that would need a lone stud (a 1×1) is listed in the notes
instead.
