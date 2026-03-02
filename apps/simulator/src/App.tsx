import { Suspense, useEffect, useRef, useState, useCallback } from "react";
import init, { Simulator } from "sim-wasm";
import wasmUrl from "sim-wasm/sim_wasm_bg.wasm?url";
import Scene from "./Scene";

export default function App() {
  const simRef = useRef<Simulator | null>(null);
  const [ready, setReady] = useState(false);
  const [depth, setDepth] = useState(1.0);
  const [stroke, setStroke] = useState(1.0);
  const [velocity, setVelocity] = useState(0.5);
  const [sensation, setSensation] = useState(0);

  useEffect(() => {
    let cancelled = false;
    init(wasmUrl).then(() => {
      if (cancelled) return;
      const sim = new Simulator(10.0);
      sim.set_depth(depth);
      sim.set_stroke(stroke);
      sim.set_velocity(velocity);
      sim.set_sensation(sensation);
      simRef.current = sim;
      setReady(true);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const updateDepth = useCallback((v: number) => {
    setDepth(v);
    simRef.current?.set_depth(v);
  }, []);

  const updateStroke = useCallback((v: number) => {
    setStroke(v);
    simRef.current?.set_stroke(v);
  }, []);

  const updateVelocity = useCallback((v: number) => {
    setVelocity(v);
    simRef.current?.set_velocity(v);
  }, []);

  const updateSensation = useCallback((v: number) => {
    setSensation(v);
    simRef.current?.set_sensation(v);
  }, []);

  if (!ready) return <p>Loading simulator…</p>;

  return (
    <div
      style={{
        fontFamily: "system-ui",
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        maxWidth: 800,
        margin: "0 auto",
      }}
    >
      <div style={{ height: 600, flexShrink: 0 }}>
        <Suspense fallback={<p style={{ padding: "1rem" }}>Loading model…</p>}>
          <Scene simulator={simRef.current!} />
        </Suspense>
      </div>

      <div style={{ flex: 1, padding: "1rem", overflowY: "auto" }}>
        <h1 style={{ fontSize: "1.25rem", marginTop: 0 }}>OSSM Simulator</h1>
        <Slider
          label="Depth"
          value={depth}
          min={0}
          max={1}
          step={0.01}
          onChange={updateDepth}
        />
        <Slider
          label="Stroke"
          value={stroke}
          min={0}
          max={1}
          step={0.01}
          onChange={updateStroke}
        />
        <Slider
          label="Velocity"
          value={velocity}
          min={0}
          max={1}
          step={0.01}
          onChange={updateVelocity}
        />
        <Slider
          label="Sensation"
          value={sensation}
          min={-100}
          max={100}
          step={1}
          onChange={updateSensation}
        />
      </div>
    </div>
  );
}

function Slider({
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
}) {
  return (
    <div style={{ marginBottom: "1rem" }}>
      <label
        style={{
          display: "flex",
          justifyContent: "space-between",
          fontSize: "0.875rem",
        }}
      >
        <span>{label}</span>
        <span>{value}</span>
      </label>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        style={{ width: "100%" }}
      />
    </div>
  );
}
