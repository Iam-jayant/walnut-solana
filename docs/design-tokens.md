# Walnut design tokens

The Walnut Web App (Task 15) reuses the **styling structure** of
[zama.org] — its type scale, spacing rhythm, grid, component shapes, and
motion — while all copy, the logo, and imagery are Walnut's own. This document
records the tokens; they are implemented as Tailwind v4 `@theme` variables in
`web/app/globals.css`, which is the single source of truth. Values here and in
that file must stay in sync.

> Scope note: these are a styling baseline. Exact pixel measurements should be
> re-checked against the live site when the landing page is built (Task 15.2);
> update both this file and `globals.css` together if they change.

## Palette

The five brand colors are fixed by the task. Tailwind generates
`bg-walnut-*` / `text-walnut-*` / `border-walnut-*` utilities from them.

| Token           | Hex       | Use                                  |
| --------------- | --------- | ------------------------------------ |
| `walnut-yellow` | `#FFD209` | Primary accent, CTAs, highlights     |
| `walnut-black`  | `#000000` | Primary text, high-contrast surfaces |
| `walnut-ink`    | `#2D2D2D` | Secondary text, muted surfaces       |
| `walnut-mist`   | `#F4F4F4` | Section backgrounds, hairlines       |
| `walnut-white`  | `#FFFFFF` | Page background, cards               |

Contrast: black (`#000000`) and ink (`#2D2D2D`) on white and on yellow both
exceed WCAG AA for body text. Yellow is an accent only; it is never used for
body text on white.

## Typography

**Typeface:** Telegraf, loaded locally via `next/font/local` (no third-party
font request). Three weights:

| Weight | File                      | Role                       |
| ------ | ------------------------- | -------------------------- |
| 200    | `Telegraf-UltraLight.otf` | Display / hero headlines   |
| 400    | `Telegraf-Regular.otf`    | Body, leads, sub-headings  |
| 800    | `Telegraf-UltraBold.otf`  | Section headings, emphasis |

The CSS variable `--font-telegraf` is injected by `next/font/local` and wired
into `--font-sans`, so the `font-sans` utility resolves to Telegraf with system
fallbacks.

**Type scale** (fluid with `clamp()`, sized for 360–1920px). Each maps to a
Tailwind `text-*` utility.

| Token          | Size (`clamp`)                             | Line height | Tracking |
| -------------- | ------------------------------------------ | ----------- | -------- |
| `text-display` | `clamp(2.75rem, 1.5rem + 5vw, 5.5rem)`     | 1.02        | -0.03em  |
| `text-h1`      | `clamp(2.25rem, 1.4rem + 3.2vw, 3.75rem)`  | 1.05        | -0.02em  |
| `text-h2`      | `clamp(1.75rem, 1.2rem + 1.8vw, 2.5rem)`   | 1.1         | -0.015em |
| `text-h3`      | `clamp(1.375rem, 1.1rem + 0.9vw, 1.75rem)` | 1.2         | —        |
| `text-lead`    | `clamp(1.125rem, 1rem + 0.4vw, 1.375rem)`  | 1.5         | —        |
| `text-body`    | `1rem`                                     | 1.6         | —        |
| `text-small`   | `0.875rem`                                 | 1.5         | —        |
| `text-caption` | `0.75rem`                                  | 1.4         | 0.04em   |

## Spacing, radii, and layout

- **Spacing** uses Tailwind's default 0.25rem step; sections use large vertical
  rhythm (`py-16` mobile, `py-24` desktop).
- **Radii:** `--radius-card` = 1.25rem (cards, panels); `--radius-pill` =
  9999px (buttons, badges).
- **Content width:** `--container-page` = 80rem (1280px), applied with
  `max-w-(--container-page)` and centered with horizontal padding that grows
  from 1.5rem (mobile) to 2.5rem (desktop).
- **Grid:** 12-column mental model; card grids step 2 → 3 → 5 columns across
  the `sm` and `lg` breakpoints.

## Motion

| Token             | Value                           | Use                 |
| ----------------- | ------------------------------- | ------------------- |
| `--ease-out-expo` | `cubic-bezier(0.16, 1, 0.3, 1)` | Enter transitions   |
| `--duration-fast` | `150ms`                         | Hover / focus       |
| `--duration-base` | `250ms`                         | Standard transition |
| `--duration-slow` | `400ms`                         | Section reveals     |

All motion is disabled under `prefers-reduced-motion: reduce` by a global rule
in `globals.css` (Requirement 15.7).

[zama.org]: https://www.zama.org
