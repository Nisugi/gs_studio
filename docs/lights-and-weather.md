# Local lights and weather — design notes

Prototype: https://claude.ai/artifact/EMs3E9DCUH9biSoMUsEEmb (swamp or desert backdrop, sidewinder + lurk, torch in the bottom-left, recolourable mote of light, switchable storm / mist / rain / sand / snow).
Builds on the v1 integration request (Creature Field Environment Lighting): https://claude.ai/code/artifact/a2146e1e-8b56-4ad5-9a98-970d2035ed64

## Model
- Local lights are a third kind of light, after scene light and time of day: point lights placed in the scene. Each adds, in linear light, on top of the v1 night result:
  `night_lit + albedo × colour(t) × power·f(t) / (d² + r0²) × window(d/range) × (max(0, (n·l + wrap)/(1 + wrap)) × shadow + bounce)`, plus a Fresnel sheen weighted toward dark (wet) pixels.
- albedo = backdrop as painted; creature after its v1 scene-light pass for that backdrop. The v1 per-pixel function is unchanged.
- Night for a lit scene: per-scene override `[light.night] intensity = 1.0, exposure = 0.07` (shipped night is 0.30).

| | Torch | Mote of light |
|---|---|---|
| power | 9 | 7 |
| r0 | 0.55 m | 0.9 m (softer) |
| range | 14 m | 10 m (Reach slider 2–20) |
| wrap (creatures / ground) | 0.10 / 0 | 0.35 / 0.25 |
| bounce (area glow) | 0.04 | 0.20 |
| shadow light radius | 0.10 m | 0.22 m (softer) |
| colour | fixed: #ffae66, hot #ffc98f, cool #ff8845 | user: base / brighter / dimmer pickers + presets |
| motion | fire flicker + ≤4 cm sway | slow pulse, 4 cm bob |

- Mote presets (dim / base / bright): Golden #ffab40 / #ffd27a / #fff0c0; Arcane #3a4dff / #6fa8ff / #cfe3ff; Ember #e0501a / #ff8a3d / #ffc27a; Void #6a2bd9 / #a66bff / #e3ccff.
- Mote pulse: n = 0.62·n(0.8s) + 0.28·n(2.1s) + 0.10·n(5.3s), s = t × speed; k = clamp(n/0.75, −1, 1); brightness = 1 + depth·k (depth default ±18%); colour = mix(base, bright or dim, |k| × colour shift) (shift default 60%).

## Backdrops
- Swamp: horizon row 262 of 848; dark painted pixels count as water (sheen, splash rings, no snow cover).
- Desert: horizon row 280; no water. Depth made with Depth Anything V2 small: whole frame + two overlapping square crops fitted by scale and offset, blended 35/65; sky (above horizon and blue) set to 0; white = near.
- Switching backdrop swaps camera, backdrop textures, the creatures' scene-lit albedo (v1 pass 1 re-baked per backdrop) and their pass-2 frames, and resets positions. Dust colour = 50/50 mix of #c9a472 and the backdrop's lower-40% ground colour (swamp #a98e68, desert #cba06e).

## Weather (each element toggles independently)
- **Storm (distant lightning).** Strikes at random times (default 7/min), 1–4 strobes (25 ms rise, 70–220 ms decay by distance), an afterglow, and a slow rumble (0.08, ~0.35 Hz noise). Signal L ≤ 1.6; flash = L × strength (default 60%).
  - Night exposure × (1 + 2.2 × flash); key and ambient tints move toward #c9d4ff by min(1, 0.6 × flash).
  - Extra light in far haze (smoothstep 10–50 m) and in the cloud above the strike (random x, top of frame).
  - Creature tops and backs catch it: albedo × flash × 0.10 × max(0, n·L) with L up-and-behind.
  - Distance slider (near → far): softer peaks (×1 → ×0.55), slower decay, more afterglow.
- **Air medium (mist, sand dust, blizzard haze).** One ray-march, 12 steps along each pixel's view ray to its surface (capped 30 m), captured at each creature's depth so farther creatures sit deeper in it. In-scattered light = night haze colour + torch + mote + lightning, tinted per component.
  - Mist: D × exp(−height / H) × drifting 3D noise; D = 0.35 × slider (default 40%), H default 0.8 m, drift 0.4 m/s. Grey.
  - Sand dust: (0.015 + 0.30 × amount) × gust × exp(−height / (1 + 5 × amount)) × streaming noise; sand-coloured (dust × 1.8). Gust fronts are 2D noise bands travelling with the wind; they fill in toward a constant storm as amount² → 1. Wind speed = (3 + 15 × |wind|) × (0.8 + 0.5 × amount) m/s.
  - Blizzard haze: (0.004 + 0.16 × amount²) × streaming noise; white (× 1.25).
- **Rain.** Streaks on seven slabs at 1.7–13.5 m, hidden wherever the scene's nearest surface is closer. Fall 7.5–10 m/s, streak = 28 ms of fall, wind slants it. Faint in the dark, bright near the lights and in lightning. Splashes within 22 m on the camera's ground plane (rings on water, crowns on stone), anti-aliased from screen derivatives. Darkens the backdrop by up to 20%, raises sheen, adds a little haze.
- **Sand.** Strength labels: gusts < 25%, blowing sand < 55%, dust storm < 80%, sandstorm. Grains: short streaks (8–22 ms of travel) along the wind on seven slabs at 1.6–12.5 m, probability and brightness weighted by exp(−height / (0.35 + 2.2 × amount)) and by the gust field. Ground: anisotropic noise streaming along the wind on up-facing ground within 30 m, faded where a pixel covers more than ~5 cm of ground.
- **Snow.** Snowfall labels: gentle < 25%, steady < 55%, heavy < 80%, blizzard. Flakes on seven slabs at 1.4–12.5 m, radius 2–7 mm, fall 0.9–2.6 m/s, sideways wind up to (0.6 + 11 × amount²) × wind m/s, wobble, motion blur of 1/160 s, nearest flakes soft and dim (out of focus). Cover slider: snow drifts (noise threshold) on ground facing up and on creature tops (n·up), not on water; applied to albedo before the night pass so the lights warm it.

## Geometry from depth maps (new per-image input)
- Backdrop depth PNG: calibrated against the scene camera's ground plane (per-row median depth value ↔ true ground distance) → value→metres lookup. Camera: 40° vertical FOV, eye height 2.2 m, horizon per backdrop.
- Creature depth PNG: relief behind the feet (feet sit on the ground at the camera's ground distance), plus a tube profile rounding silhouette edges. Normals from one-sided derivatives. Geometry is in metres, so it does not depend on the backdrop or placement.
  - Snake (painted depth, flat per part): relief 1.1 m, depth steps also count as edges.
  - Lurk (estimated depth): relief 0.6 m, silhouette edges only, tube gain 0.6. Depth Anything V2 small: whole figure padded square + top and bottom halves.
- Emissive mask (lurk's eyes): those pixels skip night pass and lights. Stored in the blue channel of the creature depth texture.
- Wound/overlay layers must borrow the base's depth and normals through the layer maps, which now need x as well as y.

## Shadows
- Every creature casts onto the backdrop, itself and other creatures, for every light; the painted backdrop does not shadow itself.
- Screen-space march from each pixel toward each light, clipped to each creature's screen rectangle, 32 steps, thickness 0.7 m, bias 0.05 m, self-shadow start 0.15 m. No random jitter (it speckled the penumbras).

## Flicker (torch)
- f(t) = 1 + 0.10·n(1.3t) + 0.08·n(4.7t) + 0.05·n(11t) + gutter; range 0.66–1.215 (10 min sample). Colour warms above 1, reddens below.

## Measured
- Shader vs Python reference (torch only): backdrop mean ΔE 0.0003; snake 0.0045 after a 4 px blur (difference is texture resampling).
- Flicker without a custom shader (bake torch light map per layer, draw with egui additive Color32 + sRGB-encoded per-frame tint): mean ΔE 0.0005–0.017, p99 ≤ 0.048 vs per-frame lighting across the flicker range.
- Flame sway of 4.8 cm changes ~1% of lit pixels by ΔE > 0.02, so a baked (static) shadow loses almost nothing.

## Open
- Confirm Vellum's scene camera fields (focal, eye_height, horizon) can supply the ground plane used here.
- Moving lights, many independent lights, and all animated weather (mist, rain, sand, snow) need the shader route (egui_wgpu::Callback).
- The storm alone might ride v1's keyframe cross-fade (bake the night look and a lightning look, cross-fade by the flash value every frame). Untested.
- A mote that floats and bobs moves its shadows every frame; on the baked route its bob would have to be dropped or kept tiny.
- Cost: every weather element adds a loop to the fragment shader; with several on, check frame time on a low-end GPU (software-rendered timings here are not representative).
