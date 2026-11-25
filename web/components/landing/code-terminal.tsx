"use client";

import React, { useState } from "react";

const DOCS_HREF = "https://github.com/Iam-jayant/walnut-solana";

export function CodeTerminal() {
  const [activeTab, setActiveTab] = useState<"ts" | "sol">("ts");

  return (
    <section
      id="developers"
      className="relative py-24 md:py-36 bg-[#0B0B0D] text-white overflow-hidden"
      style={{
        backgroundImage: `
          radial-gradient(ellipse 70% 60% at 30% 0%, rgba(255, 220, 80, 0.14) 0%, rgba(220, 160, 30, 0.04) 40%, transparent 70%),
          radial-gradient(circle 500px at 75% 25%, rgba(255, 224, 82, 0.06) 0%, transparent 60%)
        `,
      }}
    >
      {/* Ambient background glow / lens flare */}
      <div
        className="pointer-events-none absolute -top-24 left-1/4 h-[350px] w-[500px] -translate-x-1/2 rounded-full opacity-30 blur-[120px]"
        style={{
          background: "radial-gradient(circle, #FFE052 0%, rgba(255, 180, 40, 0.4) 60%, transparent 100%)",
        }}
      />

      <div className="relative z-10 mx-auto max-w-5xl px-6 md:px-10">
        {/* Section Header matching Zama exact styling */}
        <div className="text-center mb-12 sm:mb-14">
          <h2
            className="text-3xl sm:text-4xl lg:text-[46px] tracking-tight leading-[1.12] mb-4 select-none"
            style={{
              fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
              fontWeight: 200,
            }}
          >
            <span className="bg-gradient-to-r from-[#FDE89C] via-[#FFF3D1] to-white bg-clip-text text-transparent">
              Integrate confidentiality in a
            </span>
            <br />
            <span className="text-white">few lines of code</span>
          </h2>
          <p className="text-xs sm:text-[14px] text-white/60 max-w-lg mx-auto leading-relaxed font-normal">
            Whether you&apos;re building confidential apps or integrating confidential lending, start with our SDK and docs.
          </p>
        </div>

        {/* Code Container matching Zama exact dark box aesthetic */}
        <div className="max-w-4xl mx-auto rounded-[14px] sm:rounded-[18px] bg-[#141416]/95 border border-white/[0.08] shadow-[0_25px_65px_-15px_rgba(0,0,0,0.85)] overflow-hidden backdrop-blur-xl">
          {/* Header Bar: Left Label/Tabs + Right Solid Yellow Docs Button */}
          <div className="flex items-center justify-between px-5 sm:px-7 py-3.5 border-b border-white/[0.06] bg-[#18181A]/90 select-none">
            <div className="flex items-center gap-3">
              <span className="text-xs sm:text-[13px] font-sans font-medium text-white/85">
                {activeTab === "ts" ? "TypeScript SDK — Confidential Lending" : "Rust Anchor — Program State Transition"}
              </span>
              <div className="h-3 w-px bg-white/20 hidden sm:block" />
              <div className="flex items-center gap-1 bg-white/[0.04] p-0.5 rounded-[6px] border border-white/[0.06]">
                <button
                  type="button"
                  onClick={() => setActiveTab("ts")}
                  className={`text-[11px] px-2.5 py-0.5 rounded-[4px] font-medium transition cursor-pointer ${
                    activeTab === "ts" ? "bg-white/15 text-white" : "text-white/40 hover:text-white/70"
                  }`}
                >
                  TS
                </button>
                <button
                  type="button"
                  onClick={() => setActiveTab("sol")}
                  className={`text-[11px] px-2.5 py-0.5 rounded-[4px] font-medium transition cursor-pointer ${
                    activeTab === "sol" ? "bg-white/15 text-white" : "text-white/40 hover:text-white/70"
                  }`}
                >
                  Rust
                </button>
              </div>
            </div>

            {/* Solid Yellow Docs Button matching Image 1 */}
            <a
              href={DOCS_HREF}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 bg-[#FED022] hover:bg-[#ffda33] text-black text-xs font-semibold px-3 py-1 sm:px-3.5 sm:py-1.5 rounded-[6px] transition cursor-pointer shadow-xs active:scale-[0.98]"
            >
              <span>Docs</span>
              <span className="text-[11px]">↗</span>
            </a>
          </div>

          {/* Syntax Highlighted Code Body with Line Numbers */}
          <div className="p-5 sm:p-7 overflow-x-auto font-mono text-[12.5px] sm:text-[13px] leading-[1.75] text-[#E6EDF3]">
            {activeTab === "ts" ? (
              <div className="table w-full">
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px] w-8">1</span>
                  <span className="table-cell text-[#6B7280]">// Confidential lending on Solana Devnet via Arcium MPC</span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">2</span>
                  <span className="table-cell">
                    <span className="text-[#FF7B72]">import</span> <span className="text-[#E6EDF3]">{'{'}</span> <span className="text-[#79C0FF]">WalnutClient</span> <span className="text-[#E6EDF3]">{'}'}</span> <span className="text-[#FF7B72]">from</span> <span className="text-[#7EE787]">&quot;@walnut/sdk&quot;</span><span className="text-[#E6EDF3]">;</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">3</span>
                  <span className="table-cell">
                    <span className="text-[#FF7B72]">import</span> <span className="text-[#E6EDF3]">{'{'}</span> <span className="text-[#79C0FF]">Connection</span><span className="text-[#E6EDF3]">,</span> <span className="text-[#79C0FF]">PublicKey</span> <span className="text-[#E6EDF3]">{'}'}</span> <span className="text-[#FF7B72]">from</span> <span className="text-[#7EE787]">&quot;@solana/web3.js&quot;</span><span className="text-[#E6EDF3]">;</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">4</span>
                  <span className="table-cell text-transparent">&nbsp;</span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">5</span>
                  <span className="table-cell text-[#6B7280]">// Initialize Walnut on Solana Devnet (Cluster 456)</span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">6</span>
                  <span className="table-cell">
                    <span className="text-[#FF7B72]">const</span> <span className="text-[#E6EDF3]">client</span> <span className="text-[#FF7B72]">=</span> <span className="text-[#FF7B72]">new</span> <span className="text-[#79C0FF]">WalnutClient</span><span className="text-[#E6EDF3]">({'{'}</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">7</span>
                  <span className="table-cell pl-4">
                    <span className="text-[#E6EDF3]">connection</span><span className="text-[#FF7B72]">:</span> <span className="text-[#FF7B72]">new</span> <span className="text-[#79C0FF]">Connection</span><span className="text-[#E6EDF3]">(</span><span className="text-[#7EE787]">&quot;https://api.devnet.solana.com&quot;</span><span className="text-[#E6EDF3]">),</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">8</span>
                  <span className="table-cell pl-4">
                    <span className="text-[#E6EDF3]">programId</span><span className="text-[#FF7B72]">:</span> <span className="text-[#FF7B72]">new</span> <span className="text-[#79C0FF]">PublicKey</span><span className="text-[#E6EDF3]">(</span><span className="text-[#7EE787]">&quot;5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea&quot;</span><span className="text-[#E6EDF3]">),</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">9</span>
                  <span className="table-cell pl-4">
                    <span className="text-[#E6EDF3]">clusterOffset</span><span className="text-[#FF7B72]">:</span> <span className="text-[#79C0FF]">456</span><span className="text-[#E6EDF3]">,</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">10</span>
                  <span className="table-cell"><span className="text-[#E6EDF3]">{'}'});</span></span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">11</span>
                  <span className="table-cell text-transparent">&nbsp;</span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">12</span>
                  <span className="table-cell text-[#6B7280]">// Supply 1,000 USDC confidentially into the lending pool</span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">13</span>
                  <span className="table-cell">
                    <span className="text-[#FF7B72]">const</span> <span className="text-[#E6EDF3]">{'{'} txSignature {'}'}</span> <span className="text-[#FF7B72]">=</span> <span className="text-[#FF7B72]">await</span> <span className="text-[#E6EDF3]">client</span><span className="text-[#E6EDF3]">.</span><span className="text-[#D2A8FF]">supplyConfidential</span><span className="text-[#E6EDF3]">({'{'}</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">14</span>
                  <span className="table-cell pl-4">
                    <span className="text-[#E6EDF3]">amount</span><span className="text-[#FF7B72]">:</span> <span className="text-[#79C0FF]">1_000_000_000n</span><span className="text-[#E6EDF3]">,</span> <span className="text-[#6B7280]">// Encrypted onchain via Arcium MPC</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">15</span>
                  <span className="table-cell pl-4">
                    <span className="text-[#E6EDF3]">userKeypair</span><span className="text-[#FF7B72]">:</span> <span className="text-[#E6EDF3]">wallet</span><span className="text-[#E6EDF3]">,</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">16</span>
                  <span className="table-cell"><span className="text-[#E6EDF3]">{'}'});</span></span>
                </div>
              </div>
            ) : (
              <div className="table w-full">
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px] w-8">1</span>
                  <span className="table-cell text-[#6B7280]">// On-chain confidential state transition with Arcium MPC</span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">2</span>
                  <span className="table-cell">
                    <span className="text-[#FF7B72]">use</span> <span className="text-[#E6EDF3]">anchor_lang::prelude::*;</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">3</span>
                  <span className="table-cell">
                    <span className="text-[#FF7B72]">use</span> <span className="text-[#E6EDF3]">walnut::state::{'{'}</span><span className="text-[#79C0FF]">LendingPool</span><span className="text-[#E6EDF3]">,</span> <span className="text-[#79C0FF]">Position</span><span className="text-[#E6EDF3]">{'}'};</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">4</span>
                  <span className="table-cell text-transparent">&nbsp;</span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">5</span>
                  <span className="table-cell"><span className="text-[#FF7B72]">#[program]</span></span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">6</span>
                  <span className="table-cell">
                    <span className="text-[#FF7B72]">pub mod</span> <span className="text-[#79C0FF]">walnut</span> <span className="text-[#E6EDF3]">{'{'}</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">7</span>
                  <span className="table-cell pl-4">
                    <span className="text-[#FF7B72]">pub fn</span> <span className="text-[#D2A8FF]">supply_confidential</span><span className="text-[#E6EDF3]">(</span><span className="text-[#E6EDF3]">ctx</span><span className="text-[#FF7B72]">:</span> <span className="text-[#79C0FF]">Context</span><span className="text-[#E6EDF3]">&lt;</span><span className="text-[#79C0FF]">Supply</span><span className="text-[#E6EDF3]">&gt;,</span> <span className="text-[#E6EDF3]">amount</span><span className="text-[#FF7B72]">:</span> <span className="text-[#79C0FF]">Enc</span><span className="text-[#E6EDF3]">&lt;</span><span className="text-[#79C0FF]">Mxe</span><span className="text-[#E6EDF3]">&gt;)</span> <span className="text-[#FF7B72]">-&gt;</span> <span className="text-[#79C0FF]">Result</span><span className="text-[#E6EDF3]">&lt;()&gt; {'{'}</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">8</span>
                  <span className="table-cell pl-8 text-[#6B7280]">
                    // Applies encrypted balance update in ciphertext via Arcium MPC
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">9</span>
                  <span className="table-cell pl-8">
                    <span className="text-[#E6EDF3]">ctx.accounts.pool.</span><span className="text-[#D2A8FF]">apply_lend_op</span><span className="text-[#E6EDF3]">(amount)?;</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">10</span>
                  <span className="table-cell pl-8">
                    <span className="text-[#79C0FF]">Ok</span><span className="text-[#E6EDF3]">(())</span>
                  </span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">11</span>
                  <span className="table-cell pl-4"><span className="text-[#E6EDF3]">{'}'}</span></span>
                </div>
                <div className="table-row">
                  <span className="table-cell pr-4 sm:pr-6 text-right select-none text-[#555] font-mono text-[12px]">12</span>
                  <span className="table-cell"><span className="text-[#E6EDF3]">{'}'}</span></span>
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
