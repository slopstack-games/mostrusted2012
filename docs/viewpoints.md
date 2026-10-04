# Viewpoints

Fixed camera poses for visual checks with `--camera x,y,z,look_x,look_y,look_z --screenshot out.png`.
Screenshots contain game content, so keep them local (session scratchpad), never in the repo.
Use `--camera=` when the first value is negative.

| name | args | shows |
|------|------|-------|
| road-cliff | `--camera=-1131,27,1627,-1190,15,1720` | road (colour + detail map, lane lines), terrain splat, trees, cliff, bridge, water |
| city-overview | *(default, no `--camera`)* | camera 250 m from the median of all placements, looking at downtown |
| car-front-left | `--vehicle 911 --camera 4.5,1.4,4.0,0,0.4,0` | paint, wheels (left side, unmirrored) |
| car-right | `--vehicle 911 --camera=-4.0,1.0,2.5,0,0.4,0` | mirrored right-hand wheels |
| drive-911 | `--drive 911 --units 12 --auto-throttle 0.4 --screenshot-delay 400` | car drives straight from the default spawn along the gravel shoulder beside the fence (before the 2026-10-04 collision fix it snagged on a phantom bow-tie triangle at z ≈ 1716) |
| city-lod-check | `--camera=-650,260,1850,-1000,60,2150` | lake, terrain, buildings; compare with `--keep-lod-proxies` to see the LOD2 overlap fix |
| downtown-plaza | `--drive 911 --units 30 --spawn=815,60,1760,270 --screenshot-delay 120` | street-level downtown via drive spawn |
| props-roadside | `--camera=-1145,21,1712,-1157.5,19,1728 --mute` | props: tyres, barrels, pallet, sign beside the guard rail (full map, ~25 s load) |
| lamp-post | `--camera=-1112,24,1640,-1130,20,1630 --mute` | compound-object street lamp upright next to a tree |
| grass-tufts | `--camera=-1130,26.5,1599,-1135.5,24.5,1608 --mute` | groundcover: dark grass tufts on a lawn next to a wall (full map; `--no-groundcover` to compare) |
