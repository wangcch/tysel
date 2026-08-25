export function SocialCard() {
  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        padding: "58px 64px",
        color: "#f8fafc",
        background: "#090b12",
        borderBottom: "12px solid #5b5ce2",
        fontFamily: "Arial, sans-serif",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 18 }}>
          <div
            style={{
              width: 30,
              height: 30,
              display: "flex",
              background: "#5b5ce2",
              transform: "rotate(45deg)",
            }}
          />
          <span style={{ fontSize: 34, fontWeight: 700, letterSpacing: "-0.04em" }}>
            Tysel
          </span>
        </div>
        <span
          style={{
            color: "#a8b0c0",
            fontSize: 18,
            letterSpacing: "0.12em",
          }}
        >
          NATIVE TYPESCRIPT RUNTIME
        </span>
      </div>

      <div style={{ display: "flex", flex: 1, alignItems: "center", gap: 48 }}>
        <div style={{ display: "flex", flex: 1, flexDirection: "column" }}>
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              fontSize: 70,
              fontWeight: 700,
              lineHeight: 1.04,
              letterSpacing: "-0.055em",
            }}
          >
            <span>Write TypeScript.</span>
            <span style={{ color: "#8d8eff" }}>Ship a binary.</span>
          </div>
          <span style={{ marginTop: 26, color: "#a8b0c0", fontSize: 25 }}>
            Services and agents. No Node.js in production.
          </span>
        </div>

        <div
          style={{
            width: 390,
            display: "flex",
            flexDirection: "column",
            border: "1px solid #303747",
            background: "#0f131d",
            fontFamily: "monospace",
            fontSize: 21,
          }}
        >
          <div
            style={{
              display: "flex",
              padding: "14px 18px",
              color: "#7f899c",
              borderBottom: "1px solid #303747",
            }}
          >
            release
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 13, padding: 22 }}>
            <span>
              <span style={{ color: "#8d8eff" }}>$</span> tysel build
            </span>
            <span style={{ color: "#cbd2df" }}>+ one executable</span>
            <span style={{ color: "#cbd2df" }}>+ explicit capabilities</span>
            <span style={{ color: "#cbd2df" }}>+ durable resume</span>
          </div>
        </div>
      </div>

      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          color: "#7f899c",
          fontSize: 18,
        }}
      >
        <span>tysel.dev</span>
        <span>Open source · Apache-2.0</span>
      </div>
    </div>
  );
}
