# Dice art

`d4.png`, `d6.png`, `d8.png`, `d10.png`, `d12.png`, `d20.png` (768x768 RGBA) and `tray.png`
(1600x907 RGBA). Percentile uses `d10.png` twice. No numerals are baked in: the page draws every
number on the pale-gold facet of each face.

Provenance: generated 2026-11-10 with the Codex image backend (`codex-cli`) through
`df-table dev media generate` (`look=none purpose=portrait aspect=1:1 wait=true`). The d20 was
made first and attached with `refs=` as the style reference for every other die. Each picture was
made on a flat magenta (#FF00FF) ground. A one-off Pillow/numpy step turned it into alpha
(alpha from the magenta excess `min(R,B)-G`, colour un-mixed from the key, crop to the die, fit to
80 percent of 768 px, soft drop shadow from the upper left added into the alpha). Edges were checked
over dark and light grounds. No secrets are recorded.

Common prompt tail: match the reference in material and style, deep ink-navy polished enamel body,
thin brushed-gold beveled edges, a flat plain pale-gold inset panel on every visible face, no
numbers or marks, key light from the upper left, soft depth, centred at about 72 percent of the
frame, flat pure magenta background with no shadow, no hard black outline.

| File | Pose asked |
| --- | --- |
| `d20.png` | Icosahedron, one triangle facing the viewer in the centre, tipped up, hexagon silhouette, about nine faces. |
| `d4.png` | Tetrahedron on one face seen from slightly above, three faces around the apex, symmetrical. |
| `d6.png` | Cube, corner toward the viewer, three faces, slightly rounded gold-beveled edges. |
| `d8.png` | Octahedron, top vertex up, four faces around it in the upper half, lower half beneath. |
| `d10.png` | Pentagonal trapezohedron, apex up, one kite face front and centre, kites left and right, lower kites below the zigzag equator. |
| `d12.png` | Dodecahedron, one pentagon tipped toward the viewer at the centre top, ring of neighbours. |
| `tray.png` | Top-down dark espresso leather dice tray, thin brushed-gold inlaid rim inside a padded leather border, empty flat middle, no marks; generated with `purpose=plate aspect=16:9`, transparent corners, no added shadow. |
