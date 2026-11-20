# studforge shape spec (version 1)

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

## How to work
1. Look at the image. Pick the mode. Note the 3–6 features that make the
   subject recognisable (silhouette, proportions, colour regions, eyes).
2. Block the big masses first (body, head), then limbs, then paint details.
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
  studs and plates; slopes fall toward -Y.
- `make_custom_part_from_commands` for anything else, written as a SolidCraft
  command batch in millimetres (8 mm pitch, 9.6 mm brick, Ø4.8 × 1.8 mm studs,
  Ø6.51/4.8 mm tubes, 1.2 mm walls, 0.1 mm side clearance) with an `expect`
  block for the volume and bounding box.

Export already writes printable files for never-produced and rare colours.
