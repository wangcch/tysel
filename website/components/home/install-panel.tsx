"use client";

import { CopyButton } from "@/components/copy-button";

const installCommand = "curl -fsSL https://tysel.dev/install.sh | sh";

export function InstallPanel() {
  return (
    <div className="max-w-xl">
      <div className="flex items-center border border-fd-border bg-tysel-ink text-tysel-white">
        <pre className="min-w-0 flex-1 overflow-x-auto px-3.5 py-2.5 font-mono text-[13px] leading-5 text-white/90">
          <code>
            <span className="text-white/35 select-none">$ </span>
            {installCommand}
          </code>
        </pre>
        <CopyButton
          value={installCommand}
          className="mx-1 shrink-0 rounded-none text-white/55 hover:bg-white/10 hover:text-white"
        />
      </div>
      <p className="mt-2 text-xs leading-5 text-fd-muted-foreground">
        Then run{" "}
        <code className="font-mono text-[12px] text-fd-foreground/80">
          tysel doctor --install
        </code>
        . No Rust, Node.js, or npm on the machine.
      </p>
    </div>
  );
}
