# Tysel Brand

## Brand idea

Tysel is a lightweight native runtime for TypeScript services and agents: **Write TypeScript. Ship a binary.**

The identity is built around compression and continuity. The tightly tracked lowercase wordmark makes the letters behave like one executable unit: source enters as familiar TypeScript and leaves as a compact native artifact. There is no unrelated emblem competing with the name—the name itself is the primary mark.

The secondary `ty` mark is a direct crop of the same outlined wordmark. It is reserved for square or very small surfaces such as favicons, package avatars, and profile images.

## Visual character

- Precise, lightweight, and infrastructure-native
- Confident rather than decorative
- Compact spacing to express one-file delivery
- Monochrome-first for terminals, documentation, and developer tooling
- One bright accent for state, output, and forward motion

## Palette

| Token | Hex | Use |
| --- | --- | --- |
| Binary Ink | `#111318` | Primary dark background and logo |
| Runtime White | `#FFFFFF` | Reversed logo and high-contrast text |
| Tysel Blue | `#5B5CE2` | Primary brand accent |
| Byte Lime | `#C9FF63` | Output, completion, and small highlights |
| Runtime Mist | `#F1F3F7` | Light surfaces |

## Logo usage

- Use [`logo/tysel-wordmark.svg`](./logo/tysel-wordmark.svg) as the default signature.
- Use [`logo/tysel-wordmark-blue.svg`](./logo/tysel-wordmark-blue.svg) when an
  external image context cannot inherit `currentColor` on a light surface, such as
  the repository README.
- Use [`logo/tysel-wordmark-white.svg`](./logo/tysel-wordmark-white.svg) for the same
  situation on a dark surface (for example GitHub dark mode via `<picture>`).
  Reversed marks use Runtime White, not Runtime Mist.
- Use [`logo/tysel-mark.svg`](./logo/tysel-mark.svg) only when the full name cannot fit.
- The blue and white README marks are cropped to the ink box
  (`viewBox="8.88 18.22 147.91 71.91"`). The default `currentColor` wordmark keeps
  the 170×100 artboard for clear-space layouts.
- All wordmark SVGs are outlined paths. They contain no live text or external font dependency.
- Preserve clear space equal to the height of the wordmark's lowercase `t` crossbar.
- Recommended minimum widths: 96 px for the wordmark and 24 px for the `ty` mark.
- Use a single solid color. Do not stretch, rotate, add shadows, re-typeset, or separate the connected letters.

## GitHub surfaces

[`github/tysel-github-banner.png`](./github/tysel-github-banner.png) is the
upload-ready 1280×640 social preview. Its editable
[`SVG source`](./github/tysel-github-banner.svg) is included for future size
variants. The wordmark is the only focal symbol; the low-contrast line field
stays in the background and suggests continuous runtime flow.

[`github/tysel-readme-pipeline.svg`](./github/tysel-readme-pipeline.svg) is the
inline README diagram: source plus manifest compress into one native executable.
It uses Binary Ink, Tysel Blue, and Byte Lime, and adapts with
`prefers-color-scheme`. Do not paste the social banner into the README body.
