# Viewpoints

Fixed camera poses for visual checks with `--camera x,y,z,look_x,look_y,look_z --screenshot out.png`.
Screenshots contain game content, so keep them local (session scratchpad), never in the repo.
Use `--camera=` when the first value is negative. `--hour N` (0–24, fractions blend keyframes, default 13) picks
the time of day; `--weather` uses the Alternate (bad-weather) keyframes; the timeline is the one painted under
the camera unless `--timeline <id|default>` is given (screenshots taken before 2026-10-04 used `default`).

| name | args | shows |
|------|------|-------|
| road-cliff | `--camera=-1131,27,1627,-1190,15,1720` | road (colour + detail map + normals, lane lines), terrain splat, trees, cliff, bridge, water, sky; with `--all-backdrops` a coarse rock tile covers the lawn on the right and a slab the road bottom left |
| city-overview | *(default, no `--camera`)* | camera 250 m from the median of all placements, looking at downtown |
| car-front-left | `--vehicle 911 --camera 4.5,1.4,4.0,0,0.4,0 --screenshot-delay 60` | paint (`--paint-index`), clear coat, wheels (left side, unmirrored); the delay lets the sky reflections build |
| wheel-close | `--vehicle 911 --camera 2.2,0.6,1.9,0.8,0.35,1.2 --screenshot-delay 60` | rim normal map, caliper logo, black tyre |
| car-right | `--vehicle 911 --camera=-4.0,1.0,2.5,0,0.4,0` | mirrored right-hand wheels |
| drive-911 | `--drive 911 --units 12 --auto-throttle 0.4 --screenshot-delay 400` | car drives straight from the default spawn along the gravel shoulder beside the fence (before the 2026-10-04 collision fix it snagged on a phantom bow-tie triangle at z ≈ 1716) |
| city-lod-check | `--camera=-650,260,1850,-1000,60,2150` | lake, terrain, buildings; compare with `--all-backdrops` to see coarse terrain over the curved road and the rails (backdrop tiers stacked) |
| car-menu | `--drive 911 --units 12 --show-menu --screenshot-delay 60` | the car menu over the drive spawn, current car and its paint swatches selected |
| downtown-plaza | `--drive 911 --units 30 --spawn=815,60,1760,270 --screenshot-delay 120` | street-level downtown via drive spawn |
| props-roadside | `--camera=-1145,21,1712,-1157.5,19,1728 --mute` | props: tyres, barrels, pallet, sign beside the guard rail (full map, ~25 s load) |
| lamp-post | `--camera=-1112,24,1640,-1130,20,1630 --mute` | compound-object street lamp upright next to a tree |
| grass-tufts | `--camera=-1130,26.5,1599,-1135.5,24.5,1608 --mute` | groundcover: dark grass tufts on a lawn next to a wall (full map; `--no-groundcover` to compare) |
| airfield-arch | `--camera=600,400,-600,300,0,-2000 --mute` | southern mountains; known seam: the mountain north of the airfield is cut into an arch where zones 46/55 do not list it |
| tyre-smoke-drift | `--drive 911 --test drift --screenshot-time 9` | tyre smoke clouds behind the sliding car on the test track (`--test handbrake --screenshot-time 7.5` for the handbrake tap) |
| crash-sparks | `--drive 911 --units 12 --auto-throttle 1 --screenshot-after-impact 0.04` | the car hits the guard rail west of downtown at ~80 km/h: spark spray and wheel dust |
| spark-demo | `MW_VFX_DEMO=010000000007bd97,0.25,15` with `--camera=-1145,20.5,1712,-1157.5,19,1728 --units 6 --mute --screenshot-delay 150` | any effect by ID (here the scrape sparks) played 8 m ahead of the camera every 0.25 s |
| road-wet | `--camera=-1131,27,1627,-1190,15,1720 --units 20 --mute` (+ `--road-wetness 0` for dry, `--planar-reflection`) | road-cliff with the wet path: puddles (puddle map), damp sheen; with `--planar-reflection` the trees reflect in the road |
| road-wet-night | `--camera=-1131,27,1627,-1190,15,1720 --units 20 --mute --hour 1 --planar-reflection` | wet night road reflecting the trees; add `--weather` for the night Alternate (fog far 7500 m) |
| planter-groundcover | `--camera=1545,30,1435,1562,22.5,1456 --mute` | groundcover tufts on a raised lawn planter that has no collision (worst unit of `mwtool placement-check`, TRK_UNIT165): checker outliers there are collision gaps, not placement errors |
