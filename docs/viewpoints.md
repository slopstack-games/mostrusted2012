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
| grass-tufts | `--camera=-1130,26.5,1599,-1135.5,24.5,1608 --mute` | groundcover: grass tufts (lit, textured; black before the 2026-10-06 normal fix) on a lawn next to a wall (full map; `--no-groundcover` to compare) |
| airfield-exit | `--camera=-210,60,-2100,0,85,-2025 --mute` | the road west out of the tunnel (zone 43) towards the airfield, looking back at the portal: the hill over it with standing trees and groundcover (zone 42's backdrop `15f3021b`); with `--composed-backdrop` the trees and sprites float in the sky (2026-10-19) |
| airfield-arch | `--camera=600,400,-600,300,0,-2000 --mute` | southern mountains; known seam: the mountain north of the airfield is cut into an arch where zones 46/55 do not list it |
| tyre-smoke-drift | `--drive 911 --test drift --screenshot-time 9` | tyre smoke clouds behind the sliding car on the test track (`--test handbrake --screenshot-time 7.5` for the handbrake tap) |
| crash-sparks | `--drive 911 --units 12 --auto-throttle 1 --screenshot-after-impact 0.04` | the car hits the guard rail west of downtown at ~80 km/h: spark spray and wheel dust |
| spark-demo | `MW_VFX_DEMO=010000000007bd97,0.25,15` with `--camera=-1145,20.5,1712,-1157.5,19,1728 --units 6 --mute --screenshot-delay 150` | any effect by ID (here the scrape sparks) played 8 m ahead of the camera every 0.25 s |
| road-wet | `--camera=-1131,27,1627,-1190,15,1720 --units 20 --mute` (+ `--road-wetness 0` for dry, `--planar-reflection`) | road-cliff with the wet path: puddles (puddle map), damp sheen; with `--planar-reflection` the trees reflect in the road |
| road-wet-night | `--camera=-1131,27,1627,-1190,15,1720 --units 20 --mute --hour 1 --planar-reflection` | wet night road reflecting the trees; add `--weather` for the night Alternate (fog far 7500 m) |
| planter-groundcover | `--camera=1545,30,1435,1562,22.5,1456 --mute` | groundcover tufts on a raised lawn planter that has no collision (worst unit of `mwtool placement-check`, TRK_UNIT165): checker outliers there are collision gaps, not placement errors |
| water-gorge | `--camera=1000,-25,3080,1150,-36,3150 --units 20 --mute --hour 12` | river water (Water_Opaque_Singlesided, `d0015e61`, y −36.8) at the bottom of the gorge under the suspension bridge: waves, surf along the banks, sky reflection; `--hour 1.5` for night. Replaces `water-river` (1610, 22, 2935), which was inside a hill looking at a flood plane (research log 2026-10-16) |
| water-industrial | `--camera=-150,80,3300,50,10,3560 --mute --hour 13` (full map) | the coastal industrial area that a far sea plane (y 30.4) used to flood: must be dry |
| water-pond | `--camera=-650,60,1850,-1000,40,2150 --mute --hour 12` | park pond from above: algae patches, floor near the banks, sky reflection in the middle |
| terrain-road | `--camera=-860,75,3170,-919.5,20,3232.5 --units 20` | slip road and junction in unit 11; with `MW_ALL_PASSES=1` a coarse grass hill (pass-mask stand-in `11e76144`) covers the road (fixed 2026-10-16) |
| surface-cobble-lane | `--camera=-347,40,1440,-347,34,1452 --units 12` | red cobbled lane (surface 25, `SurfacePhysics` 510001) between grey paving (2, 406000) at a road (the triangles: `mwtool surfaces --near=-347,1452,25 --tris`) |
| surface-cobble-drive | `--drive 911 --units 12 --spawn=859.8,39.3,1909.9,137.3 --auto-throttle 0.5 --no-traffic --screenshot-delay 300` | the 911 across the cobbled plaza (surface 25, 8150 m²): kick-up `…0f565f` behind the wheels; `MW_SURFACE_LOG=1` for the wheels' surfaces |
| surface-paving-drive | `--drive 911 --units 12 --spawn=-33.7,38.6,-83.8,41.9 --auto-throttle 0.5 --no-traffic --screenshot-delay 300` | paving (surface 2, 406000: grip 1, rolling 0.025, slide drag 1.5) |
| surface-dirt-drive | `--drive 911 --units 12 --spawn=-1216.6,27.2,1845.6,124.3 --auto-throttle 0.6 --no-traffic --screenshot-delay 300` | onto dirt (surface 7, 283003) from the road edge; the dirt smoke `…0a39d6` |
| surface-grass-drive | `--drive 911 --units 12 --spawn=1075.9,20.1,41.0,104.8 --auto-throttle 0.5 --no-traffic --screenshot-delay 300` | park lawn (surface 4, 283005, 12 000 m²) |
| terrain-road-2 | `--camera=250,80,-1930,212.5,47.6,-1974.5` | four carriageways beside a cliff; with `MW_ALL_PASSES=1` a coarse rock tile covers them |
