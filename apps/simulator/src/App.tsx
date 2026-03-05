import { Suspense, useRef, useState, useCallback, useMemo } from "react";
import { useSimulator } from "./hooks/useSimulator";
import { useAppearance } from "./hooks/useAppearance";
import { useIsMobile } from "./hooks/useIsMobile";
import {
  Theme,
  Box,
  Card,
  Flex,
  Heading,
  Text,
  Slider,
  Spinner,
  Select,
  Button,
  IconButton,
  ScrollArea,
  Separator,
  Tooltip,
} from "@radix-ui/themes";
import { SunIcon, MoonIcon, ResetIcon } from "@radix-ui/react-icons";
import Scene from "./Scene";
import type { SceneHandle } from "./Scene";

type PlaybackState = "stopped" | "playing" | "paused";

export default function App() {
  const simulator = useSimulator();
  const sceneRef = useRef<SceneHandle>(null);
  const [depth, setDepth] = useState(1.0);
  const [stroke, setStroke] = useState(1.0);
  const [velocity, setVelocity] = useState(0.5);
  const [sensation, setSensation] = useState(0.0);
  const [selectedPattern, setSelectedPattern] = useState(0);
  const [playbackState, setPlaybackState] = useState<PlaybackState>("stopped");
  const [appearance, toggleAppearance] = useAppearance();
  const isMobile = useIsMobile();

  const patterns = useMemo<
    { index: number; name: string; description: string }[]
  >(() => {
    const count = simulator.pattern_count();
    return Array.from({ length: count }, (_, i) => ({
      index: i,
      name: simulator.pattern_name(i),
      description: simulator.pattern_description(i),
    }));
  }, [simulator]);

  const updateDepth = useCallback(
    (v: number) => {
      setDepth(v);
      simulator.set_depth(v);
    },
    [simulator],
  );

  const updateStroke = useCallback(
    (v: number) => {
      setStroke(v);
      simulator.set_stroke(v);
    },
    [simulator],
  );

  const updateVelocity = useCallback(
    (v: number) => {
      setVelocity(v);
      simulator.set_velocity(v);
    },
    [simulator],
  );

  const updateSensation = useCallback(
    (v: number) => {
      setSensation(v);
      simulator.set_sensation(v);
    },
    [simulator],
  );

  const handlePlay = useCallback(() => {
    simulator.play(selectedPattern);
    setPlaybackState("playing");
  }, [simulator, selectedPattern]);

  const handlePause = useCallback(() => {
    simulator.pause();
    setPlaybackState("paused");
  }, [simulator]);

  const handleResume = useCallback(() => {
    simulator.resume();
    setPlaybackState("playing");
  }, [simulator]);

  const handleStop = useCallback(() => {
    simulator.stop();
    setPlaybackState("stopped");
  }, [simulator]);

  const handlePatternChange = useCallback(
    (value: string) => {
      const index = Number(value);
      setSelectedPattern(index);
      simulator.play(index);
      setPlaybackState("playing");
    },
    [simulator],
  );

  return (
    <Theme accentColor="purple" radius="large" appearance={appearance}>
      <Flex direction={isMobile ? "column" : "row"} height="100vh">
        <Box
          style={{
            flex: isMobile ? undefined : 1,
            height: isMobile ? "30vh" : "100vh",
            minHeight: 0,
          }}
        >
          <Suspense
            fallback={
              <Flex align="center" justify="center" height="100%" gap="3">
                <Spinner size="3" />
                <Text size="2">Loading model…</Text>
              </Flex>
            }
          >
            <Scene ref={sceneRef} simulator={simulator} />
          </Suspense>
        </Box>

        <Box
          style={{
            width: isMobile ? undefined : "360px",
            height: isMobile ? "70vh" : "100vh",
            flexShrink: 0,
          }}
          p="3"
        >
          <Card
            size="2"
            style={{
              height: "100%",
              display: "flex",
              flexDirection: "column",
            }}
          >
            <Flex justify="between" align="center" mb="3">
              <Heading size="5">OSSM Simulator</Heading>
              <Tooltip
                content={
                  appearance === "light"
                    ? "Switch to dark mode"
                    : "Switch to light mode"
                }
              >
                <IconButton
                  variant="ghost"
                  size="2"
                  onClick={toggleAppearance}
                  aria-label="Toggle theme"
                >
                  {appearance === "light" ? <SunIcon /> : <MoonIcon />}
                </IconButton>
              </Tooltip>
            </Flex>

            <Separator size="4" mb="3" />

            <ScrollArea style={{ flex: 1 }} scrollbars="vertical">
              <Flex direction="column" gap="4" pr="1">
                <Box>
                  <Text size="2" weight="medium" mb="1" as="label">
                    Pattern
                  </Text>
                  <Select.Root
                    value={String(selectedPattern)}
                    onValueChange={handlePatternChange}
                  >
                    <Select.Trigger style={{ width: "100%" }} />
                    <Select.Content>
                      {patterns.map((p) => (
                        <Select.Item key={p.index} value={String(p.index)}>
                          {p.name}
                        </Select.Item>
                      ))}
                    </Select.Content>
                  </Select.Root>
                  {patterns[selectedPattern] && (
                    <Text size="1" color="gray" mt="1">
                      {patterns[selectedPattern].description}
                    </Text>
                  )}
                </Box>

                <Flex gap="2">
                  {playbackState !== "playing" ? (
                    <Button
                      onClick={
                        playbackState === "paused" ? handleResume : handlePlay
                      }
                      variant="solid"
                    >
                      {playbackState === "paused" ? "Resume" : "Play"}
                    </Button>
                  ) : (
                    <Button onClick={handlePause} variant="soft">
                      Pause
                    </Button>
                  )}
                  <Button
                    onClick={handleStop}
                    variant="outline"
                    disabled={playbackState === "stopped"}
                  >
                    Stop
                  </Button>
                </Flex>

                <Separator size="4" />

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
                  min={-1}
                  max={1}
                  step={0.01}
                  onChange={updateSensation}
                />

                <Separator size="4" />

                <Button
                  variant="outline"
                  onClick={() => sceneRef.current?.resetView()}
                  style={{ width: "100%" }}
                >
                  <ResetIcon /> Reset View
                </Button>
              </Flex>
            </ScrollArea>
          </Card>
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
