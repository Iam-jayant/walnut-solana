# Walnut brand kit

Everything needed to represent Walnut consistently: logo files, colours, type, social assets and usage rules.

## The mark

A coin split along a walnut seam, in two tones.

- **Coin:** it is a lending and liquidity protocol.
- **Walnut seam:** the name, and a shell that opens to reveal what is inside.
- **Two halves:** private computation only works when the parties work together (multi-party computation).

Source of truth in code: [`web/components/brand-logo.tsx`](../web/components/brand-logo.tsx).

## Files

| Folder | Contents |
| :-- | :-- |
| `logo/` | Mark and lockup (mark + wordmark) in SVG and PNG. `ink` is for light backgrounds, `bone` for dark ones. `mono` versions use one colour for stamps, print and embossing. |
| `app-icon/` | App icon (512), Apple touch icon (180) and favicon (32). The live favicon is `web/app/icon.svg`. |
| `social/` | X profile picture (400 px, shipped at 800), X header 1500x500 (shipped at 3000x1000), X article cover 1500x600 (shipped at 3000x1200), link preview image 1200x630. |

SVG files that contain the word "Walnut" use the Inter font. If Inter is not installed, use the PNG files.

## Colours

Neutral on purpose. There is no brand yellow.

| Name | Hex | Use |
| :-- | :-- | :-- |
| Ink | `#111111` | Mark, text, dark backgrounds |
| Bone | `#F4F2EE` | Mark on dark, soft backgrounds |
| Stone | `#8C8880` | Second half of the mark, secondary text |
| Sand | `#D8D4CB` | Pills, highlights, diagram accents |
| White | `#FFFFFF` | Page background |

The mark always uses Ink or Bone for the left half and Stone for the right half. The mono versions use one colour for both halves.

## Typography

- **Wordmark and headlines:** Inter, Bold or ExtraBold, tight tracking (about -2%).
- **Body:** Inter Regular or Medium.
- The website currently loads Telegraf for display text. Inter is the fallback for all off-site material.

## Usage rules

- **Clear space:** keep at least half the mark's width free on every side.
- **Minimum size:** mark 16 px (favicon), lockup 96 px wide.
- **Backgrounds:** Ink mark on white or Bone. Bone mark on Ink or other dark colours. Do not place the mark on busy images.
- **Do not** recolour the halves, add outlines or shadows, rotate or stretch the mark, swap the halves, or add a yellow accent.
- **Do not** redraw the seam. Use the files or the `WalnutLogo` component.
- Use "Walnut" (not "WALNUT") in running text, and "Walnut Protocol" on first mention.

## Social

- **X profile picture:** `social/x-avatar.png`. The mark is centred so the circular crop never touches it.
- **X header:** `social/x-banner.png`, 3:1 (1500x500). X can crop 60 px from the top and bottom, and the profile picture covers the lower-left, so key content sits centre-right.
- **X article cover:** `social/x-article-cover.png`, 5:2.
- **Link preview:** `social/og-image.png`.

## Voice

Plain and direct. Short sentences. Say what it does, not how clever it is. Prefer "private lending on Solana" over jargon. Explain MPC with everyday pictures when talking to new users.
