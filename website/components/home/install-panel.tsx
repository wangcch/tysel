"use client";

import { CopyButton } from "@/components/copy-button";

const managedInstall = `curl -fsSL https://github.com/wangcch/tysel/releases/latest/download/install.sh | sh
tysel doctor --install`;

export function InstallPanel({
  compact = false,
}: {
  compact?: boolean;
}) {
  return (
    <div className="home-terminal overflow-hidden border border-fd-border bg-tysel-ink text-tysel-white">
      <div className="flex items-center justify-between gap-3 border-b border-white/10 px-3 py-2">
        <p className="text-xs font-medium text-white/70">Install Tysel</p>
        <CopyButton
          value={managedInstall}
          className="text-white/55 hover:bg-white/10 hover:text-white"
        />
      </div>
      <pre
        className={`overflow-x-auto p-4 font-mono text-[13px] leading-6 text-white/90 ${
          compact ? "max-h-40" : ""
        }`}
      >
        <code>{managedInstall}</code>
      </pre>
      <p className="border-t border-white/10 px-4 py-2 text-xs text-white/45">
        No tagged binary release yet. Use the source path until GitHub Releases
        publishes install.sh.
      </p>
    </div>
  );
}
