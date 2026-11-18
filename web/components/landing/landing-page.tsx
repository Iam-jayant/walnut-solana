import Link from "next/link";

const APP_HREF = "/app";

const NAV = [
  { href: "#problem", label: "Problem" },
  { href: "#solution", label: "Solution" },
  { href: "#features", label: "Features" },
  { href: "#how", label: "How it works" },
];

function Logo() {
  return (
    <div className="flex items-center gap-2.5">
      <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-walnut-yellow text-base font-[800] text-walnut-black">
        W
      </span>
      <span className="text-lead font-[800] tracking-tight">Walnut</span>
    </div>
  );
}

function LaunchButton({
  variant = "yellow",
  children = "Launch app",
}: {
  variant?: "yellow" | "dark";
  children?: React.ReactNode;
}) {
  const style =
    variant === "yellow"
      ? "bg-walnut-yellow text-walnut-black hover:brightness-95"
      : "bg-walnut-black text-walnut-white hover:bg-walnut-ink";
  return (
    <Link
      href={APP_HREF}
      className={`inline-flex items-center justify-center gap-2 rounded-pill px-6 py-3 text-body font-[800] transition duration-(--duration-base) ${style}`}
    >
      {children}
      <span aria-hidden>→</span>
    </Link>
  );
}

function Nav() {
  return (
    <header className="sticky top-0 z-20 border-b border-black/5 bg-walnut-white/90 backdrop-blur">
      <div className="mx-auto flex max-w-(--container-page) items-center justify-between gap-4 px-6 py-3 md:px-10">
        <Link href="/" aria-label="Walnut home">
          <Logo />
        </Link>
        <nav className="hidden items-center gap-8 text-small font-[800] md:flex">
          {NAV.map((n) => (
            <a
              key={n.href}
              href={n.href}
              className="hover:underline underline-offset-4"
            >
              {n.label}
            </a>
          ))}
        </nav>
        <Link
          href={APP_HREF}
          className="rounded-pill bg-walnut-yellow px-5 py-2 text-small font-[800] text-walnut-black transition hover:brightness-95"
        >
          Launch app
        </Link>
      </div>
    </header>
  );
}

function PositionCard() {
  const rows: { label: string; width: string }[] = [
    { label: "Collateral", width: "78%" },
    { label: "Debt", width: "46%" },
    { label: "Health factor", width: "62%" },
  ];
  return (
    <div className="animate-rise rounded-card bg-walnut-black p-6 text-walnut-white shadow-2xl md:p-8">
      <div className="flex items-center justify-between text-caption font-[800]">
        <span className="inline-flex items-center gap-2 uppercase">
          <span className="animate-scan h-2 w-2 rounded-full bg-walnut-yellow" />
          Encrypted on-chain
        </span>
        <span className="text-white/60">Arcium MPC</span>
      </div>
      <div className="mt-8 space-y-6">
        {rows.map((r) => (
          <div key={r.label}>
            <div className="flex justify-between text-small">
              <span className="font-[800]">{r.label}</span>
              <span className="animate-scan font-mono text-white/60">
                ████████
              </span>
            </div>
            <div className="mt-2 h-2 overflow-hidden rounded-pill bg-white/10">
              <div
                className="h-full rounded-pill bg-walnut-yellow"
                style={{ width: r.width }}
              />
            </div>
          </div>
        ))}
      </div>
      <p className="mt-8 text-small text-white/60">
        Only your Viewing Key can read these values.
      </p>
    </div>
  );
}

function Hero() {
  return (
    <section className="mx-auto grid max-w-(--container-page) items-center gap-12 px-6 py-20 md:px-10 lg:grid-cols-2 lg:py-28">
      <div className="animate-rise">
        <span className="inline-flex items-center gap-2 rounded-pill bg-walnut-mist px-3 py-1.5 text-caption font-[800]">
          <span className="h-2 w-2 rounded-full bg-emerald-500" />
          Live on Solana Devnet
        </span>
        <h1 className="mt-6 text-display font-[200]">
          Lend and borrow.
          <br />
          <span className="font-[800]">Keep it private.</span>
        </h1>
        <p className="mt-6 max-w-xl text-lead text-walnut-ink">
          Walnut is a lending market on Solana where balances, collateral and
          debt stay encrypted. Arcium multi-party computation runs the math, and
          nobody sees your position but you.
        </p>
        <div className="mt-10 flex flex-wrap gap-3">
          <LaunchButton />
          <a
            href="#how"
            className="inline-flex items-center rounded-pill border border-black/15 px-6 py-3 text-body font-[800] hover:bg-walnut-mist"
          >
            How it works
          </a>
        </div>
      </div>
      <PositionCard />
    </section>
  );
}

function Metrics() {
  const items = [
    "Encrypted balances",
    "Encrypted collateral",
    "Encrypted debt",
    "Private liquidation checks",
    "Viewing Key access",
    "Built on Solana",
    "Powered by Arcium",
  ];
  const doubled = [...items, ...items];
  return (
    <section
      aria-label="Highlights"
      className="overflow-hidden bg-walnut-yellow py-4 text-walnut-black"
    >
      <div className="animate-marquee flex w-max gap-12 whitespace-nowrap text-small font-[800] uppercase tracking-wide">
        {doubled.map((t, i) => (
          <span key={`${t}-${i}`}>{t}</span>
        ))}
      </div>
    </section>
  );
}

function SectionHead({
  id,
  kicker,
  title,
  body,
}: {
  id: string;
  kicker: string;
  title: string;
  body: string;
}) {
  return (
    <div id={id} className="max-w-3xl scroll-mt-24">
      <p className="text-caption font-[800] uppercase text-walnut-ink/60">
        {kicker}
      </p>
      <h2 className="mt-3 text-h1 font-[200]">{title}</h2>
      <p className="mt-5 text-lead text-walnut-ink">{body}</p>
    </div>
  );
}

function Problem() {
  const cards = [
    {
      t: "Positions are public",
      d: "On a transparent chain anyone can read your collateral, your debt and your health factor.",
    },
    {
      t: "Liquidations are hunted",
      d: "Bots watch visible health factors and race to liquidate the moment a position gets close to the line.",
    },
    {
      t: "Strategies leak",
      d: "Your size and timing become signals that others can copy or trade against.",
    },
  ];
  return (
    <section className="mx-auto max-w-(--container-page) px-6 py-20 md:px-10 md:py-28">
      <SectionHead
        id="problem"
        kicker="The problem"
        title="Open ledgers expose every position."
        body="Public lending turns personal finance into a spectator sport. Transparency helps auditing, but it should not cost you your privacy."
      />
      <div className="mt-12 grid gap-5 md:grid-cols-3">
        {cards.map((c) => (
          <div key={c.t} className="rounded-card bg-walnut-mist p-7">
            <h3 className="text-h3 font-[800]">{c.t}</h3>
            <p className="mt-3 text-body text-walnut-ink">{c.d}</p>
          </div>
        ))}
      </div>
    </section>
  );
}

function Solution() {
  const points = [
    "Your balance, collateral and debt are stored on-chain as ciphertext.",
    "Arcium nodes compute on encrypted values without ever seeing them.",
    "The program enforces solvency from encrypted state, with no trusted operator.",
    "You decrypt your own position locally with your Viewing Key.",
  ];
  return (
    <section className="bg-walnut-black py-20 text-walnut-white md:py-28">
      <div className="mx-auto grid max-w-(--container-page) gap-12 px-6 md:px-10 lg:grid-cols-2">
        <div id="solution" className="scroll-mt-24">
          <p className="text-caption font-[800] uppercase text-walnut-yellow">
            The solution
          </p>
          <h2 className="mt-3 text-h1 font-[200]">
            Private by construction, verifiable on Solana.
          </h2>
          <p className="mt-5 text-lead text-white/70">
            Walnut keeps the speed and composability of Solana and adds a
            confidentiality layer powered by Arcium multi-party computation.
          </p>
        </div>
        <ul className="space-y-4">
          {points.map((p, i) => (
            <li
              key={p}
              className="flex gap-4 rounded-card bg-walnut-ink p-5 text-body"
            >
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-walnut-yellow text-small font-[800] text-walnut-black">
                {i + 1}
              </span>
              {p}
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

function Features() {
  const items = [
    [
      "Private deposits",
      "Supply assets and earn without publishing your balance.",
    ],
    ["Private borrowing", "Open a loan against encrypted collateral."],
    [
      "Encrypted health factor",
      "Solvency is checked inside MPC, never in the clear.",
    ],
    [
      "Confidential liquidation",
      "Only positions that are actually unsafe are ever flagged.",
    ],
    ["Viewing Key", "One key you control to read your own position."],
    [
      "Self custody",
      "Funds stay in program accounts you can exit at any time.",
    ],
  ];
  return (
    <section className="mx-auto max-w-(--container-page) px-6 py-20 md:px-10 md:py-28">
      <SectionHead
        id="features"
        kicker="Features"
        title="Everything a lending market needs, minus the exposure."
        body="Deposit, borrow, repay and withdraw with the same flow you know, on private rails."
      />
      <div className="mt-12 grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
        {items.map(([t, d]) => (
          <div
            key={t}
            className="rounded-card border border-black/10 p-7 transition duration-(--duration-base) hover:-translate-y-1 hover:border-walnut-yellow hover:shadow-lg"
          >
            <span className="block h-1.5 w-10 rounded-pill bg-walnut-yellow" />
            <h3 className="mt-5 text-h3 font-[800]">{t}</h3>
            <p className="mt-3 text-body text-walnut-ink">{d}</p>
          </div>
        ))}
      </div>
    </section>
  );
}

function HowItWorks() {
  const steps = [
    ["Connect", "Link a Solana wallet and unlock your Viewing Key."],
    ["Encrypt", "Amounts are encrypted in your browser before they leave it."],
    ["Compute", "Arcium MPC nodes update your position over ciphertext."],
    ["Settle", "The result is written back to Solana. You alone can read it."],
  ];
  return (
    <section className="bg-walnut-mist py-20 md:py-28">
      <div className="mx-auto max-w-(--container-page) px-6 md:px-10">
        <SectionHead
          id="how"
          kicker="How it works"
          title="Four steps from wallet to private position."
          body="Every action follows the same encrypted round trip."
        />
        <ol className="mt-12 grid gap-5 md:grid-cols-4">
          {steps.map(([t, d], i) => (
            <li key={t} className="rounded-card bg-walnut-white p-7">
              <span className="text-h2 font-[800] text-walnut-yellow">
                {String(i + 1).padStart(2, "0")}
              </span>
              <h3 className="mt-3 text-h3 font-[800]">{t}</h3>
              <p className="mt-3 text-body text-walnut-ink">{d}</p>
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}

function Cta() {
  return (
    <section className="mx-auto max-w-(--container-page) px-6 py-20 md:px-10 md:py-28">
      <div className="rounded-card bg-walnut-yellow p-10 text-center md:p-16">
        <h2 className="text-h1 font-[800]">Your position. Your eyes only.</h2>
        <p className="mx-auto mt-4 max-w-xl text-lead">
          Open Walnut and start lending privately on Solana.
        </p>
        <div className="mt-8 flex justify-center">
          <LaunchButton variant="dark" />
        </div>
      </div>
    </section>
  );
}

function Footer() {
  return (
    <footer className="border-t border-black/5 py-10">
      <div className="mx-auto flex max-w-(--container-page) flex-wrap items-center justify-between gap-4 px-6 text-small text-walnut-ink md:px-10">
        <Logo />
        <span>Private lending on Solana, powered by Arcium MPC.</span>
        <span>Solana Devnet</span>
      </div>
    </footer>
  );
}

export function LandingPage() {
  return (
    <>
      <Nav />
      <main>
        <Hero />
        <Metrics />
        <Problem />
        <Solution />
        <Features />
        <HowItWorks />
        <Cta />
      </main>
      <Footer />
    </>
  );
}
