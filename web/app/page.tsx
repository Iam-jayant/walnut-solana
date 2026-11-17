/*
 * Design-system preview (Task 15.1). This is scaffolding, not the landing page:
 * it renders the palette, type scale, and font weights so the tokens can be
 * reviewed in isolation. The landing page, live protocol section, and docs
 * (Task 15.2) and the app screens (Task 15.4) are built once the SDK read paths
 * are available. All text here is Walnut's own.
 */

const SWATCHES: ReadonlyArray<{
  name: string;
  token: string;
  className: string;
}> = [
  { name: "Yellow", token: "#FFD209", className: "bg-walnut-yellow" },
  { name: "Black", token: "#000000", className: "bg-walnut-black" },
  { name: "Ink", token: "#2D2D2D", className: "bg-walnut-ink" },
  { name: "Mist", token: "#F4F4F4", className: "bg-walnut-mist" },
  {
    name: "White",
    token: "#FFFFFF",
    className: "bg-walnut-white ring-1 ring-walnut-mist",
  },
];

const TYPE_SAMPLES: ReadonlyArray<{ label: string; className: string }> = [
  { label: "Display", className: "text-display font-[200]" },
  { label: "Heading 1", className: "text-h1 font-[800]" },
  { label: "Heading 2", className: "text-h2 font-[800]" },
  { label: "Heading 3", className: "text-h3 font-[400]" },
  { label: "Lead", className: "text-lead font-[400]" },
  { label: "Body", className: "text-body font-[400]" },
  { label: "Small", className: "text-small font-[400]" },
];

export default function DesignSystemPage() {
  return (
    <main className="mx-auto max-w-(--container-page) px-6 py-16 md:px-10 md:py-24">
      <header className="max-w-3xl">
        <p className="text-caption font-[400] uppercase tracking-[0.04em] text-walnut-ink">
          Walnut design system
        </p>
        <h1 className="mt-4 text-display font-[200] text-walnut-black">
          Confidential lending, clearly presented.
        </h1>
        <p className="mt-6 text-lead text-walnut-ink">
          The tokens below are the shared foundation for the landing page and
          the app. Palette is fixed; the scale is fluid from 360 to 1920px.
        </p>
      </header>

      <section className="mt-16" aria-labelledby="palette">
        <h2 id="palette" className="text-h3 font-[800]">
          Palette
        </h2>
        <ul className="mt-6 grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
          {SWATCHES.map((swatch) => (
            <li key={swatch.name} className="rounded-card overflow-hidden">
              <div className={`h-24 w-full ${swatch.className}`} />
              <div className="mt-2">
                <p className="text-small font-[800]">{swatch.name}</p>
                <p className="text-caption font-mono text-walnut-ink">
                  {swatch.token}
                </p>
              </div>
            </li>
          ))}
        </ul>
      </section>

      <section className="mt-16" aria-labelledby="type">
        <h2 id="type" className="text-h3 font-[800]">
          Type scale
        </h2>
        <dl className="mt-6 space-y-6">
          {TYPE_SAMPLES.map((sample) => (
            <div
              key={sample.label}
              className="border-t border-walnut-mist pt-6"
            >
              <dt className="text-caption uppercase tracking-[0.04em] text-walnut-ink">
                {sample.label}
              </dt>
              <dd className={`mt-2 ${sample.className}`}>
                Walnut keeps your position private.
              </dd>
            </div>
          ))}
        </dl>
      </section>
    </main>
  );
}
