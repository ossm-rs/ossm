import { Suspense, useEffect, useRef, useState, useCallback } from "react";
import init, { Simulator } from "sim-wasm";
import wasmUrl from "sim-wasm/sim_wasm_bg.wasm?url";
import { Theme, Box, Flex, Heading, Text, Slider, Spinner } from "@radix-ui/themes";
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
      simRef.current = new Simulator(10.0);
      setReady(true);
    });
    return () => {
      cancelled = true;
    };
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

  if (!ready) {
    return (
      <Theme accentColor="purple" radius="large">
        <Flex align="center" justify="center" height="100vh" gap="3">
          <Spinner size="3" />
          <Text size="3">Loading simulator…</Text>
        </Flex>
      </Theme>
    );
  }

  return (
    <Theme accentColor="purple" radius="large">
      <Flex direction="column" height="100vh" maxWidth="800px" mx="auto">
        <Box flexShrink="0" height="600px">
          <Suspense
            fallback={
              <Flex align="center" justify="center" height="100%" gap="3">
                <Spinner size="3" />
                <Text size="2">Loading model…</Text>
              </Flex>
            }
          >
            <Scene simulator={simRef.current!} />
          </Suspense>
        </Box>

        <Box flexGrow="1" p="4" overflowY="auto">
          <Heading size="5" mb="4">
            OSSM Simulator
          </Heading>

          <Flex direction="column" gap="4">
            <LabeledSlider
              label="Depth"
              value={depth}
              min={0}
              max={1}
              step={0.01}
              onChange={updateDepth}
            />
            <LabeledSlider
              label="Stroke"
              value={stroke}
              min={0}
              max={1}
              step={0.01}
              onChange={updateStroke}
            />
            <LabeledSlider
              label="Velocity"
              value={velocity}
              min={0}
              max={1}
              step={0.01}
              onChange={updateVelocity}
            />
            <LabeledSlider
              label="Sensation"
              value={sensation}
              min={-100}
              max={100}
              step={1}
              onChange={updateSensation}
            />
          </Flex>
        </Box>
      </Flex>
    </Theme>
  );
}

function LabeledSlider({
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
    <Box>
      <Flex justify="between" mb="1">
        <Text size="2" weight="medium">
          {label}
        </Text>
        <Text size="2" color="gray">
          {value}
        </Text>
      </Flex>
      <Slider
        min={min}
        max={max}
        step={step}
        value={[value]}
        onValueChange={(values) => onChange(values[0])}
      />
    </Box>
  );
}
