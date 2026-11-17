import {
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { CSSProperties } from "react";
import { useSearchParams } from "react-router";
import {
  Badge,
  Box,
  Button,
  Card,
  Flex,
  Heading,
  IconButton,
  Select,
  Slider,
  Text,
} from "@radix-ui/themes";
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  ReloadIcon,
} from "@radix-ui/react-icons";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import GuideScene, { partsSeconds, stepAt, stepSeconds } from "../GuideScene";
import PartFiles from "../components/PartFiles";
import { GUIDE, GUIDE_INTRO, GUIDE_STEPS } from "../guide/ossm-alt";
import { PART_CATALOG } from "../parts";
import { useDebugTools } from "../hooks/useDebugTools";
import { useIsMobile } from "../hooks/useIsMobile";
import { indexPartOf, indexPartsFor } from "../model/catalog";
import styles from "./GuidePage.module.css";

const catalogByInstance = new Map(
  PART_CATALOG.flatMap((p) => p.instances.map((i) => [i, p] as const)),
);

function MarkdownText({ children }: { children: string }) {
  return (
    <div className={styles.markdown}>
      <Markdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: (props) => (
            <a {...props} target="_blank" rel="noreferrer noopener" />
          ),
        }}
      >
        {children}
      </Markdown>
    </div>
  );
}

/** The step's parts with quantities: by catalog name, or by part name where a
 *  catalog entry groups different parts (the top cover and its LED window). */
function StepParts({ instances }: { instances: string[] }) {
  const rows = useMemo(() => {
    const counts = new Map<string, number>();
    for (const i of instances) {
      const entry = catalogByInstance.get(i);
      const mixed = entry && indexPartsFor(entry.instances).length > 1;
      const name = (mixed ? indexPartOf(i)?.name : entry?.name) ?? i;
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
    return [...counts];
  }, [instances]);
  return (
    <Flex direction="column" gap="1">
      {rows.map(([name, n]) => (
        <Flex key={name} justify="between" gap="2">
          <Text size="2">{name}</Text>
          {n > 1 && (
            <Badge size="1" variant="soft" color="gray">
              ×{n}
            </Badge>
          )}
        </Flex>
      ))}
    </Flex>
  );
}


/** Bands per section with a tick per step; clicking a tick plays that step. */
function Timeline({
  position,
  onScrub,
  onStep,
}: {
  position: number;
  onScrub: (v: number) => void;
  onStep: (i: number) => void;
}) {
  const { index } = stepAt(position, GUIDE_STEPS.length);
  return (
    <Flex direction="column" gap="2">
      <Slider
        size="1"
        min={0}
        max={GUIDE_STEPS.length}
        step={0.001}
        value={[position]}
        onValueChange={([v]) => onScrub(v)}
        aria-label="Build progress"
      />
      <div className={styles.sections}>
        {GUIDE.map((sec) => {
          const first = GUIDE_STEPS.indexOf(sec.steps[0]);
          const active = index >= first && index < first + sec.steps.length;
          return (
            <div
              key={sec.title}
              className={`${styles.section} ${active ? styles.sectionActive : ""}`}
              style={{ flex: sec.steps.length }}
            >
              <div
                className={styles.ticks}
                style={
                  {
                    "--fill": Math.min(
                      Math.max((position - first) / sec.steps.length, 0),
                      1,
                    ),
                  } as CSSProperties
                }
              >
                {sec.steps.map((s, k) => {
                  const i = first + k;
                  const cls =
                    i === index
                      ? styles.tickCurrent
                      : i < index
                        ? styles.tickDone
                        : "";
                  return (
                    <button
                      key={s.id}
                      className={`${styles.tick} ${cls}`}
                      title={`${i + 1}. ${s.title}`}
                      aria-label={`Step ${i + 1}: ${s.title}`}
                      onClick={() => onStep(i)}
                    />
                  );
                })}
              </div>
              <Text
                size="1"
                color={active ? undefined : "gray"}
                className={styles.sectionLabel}
              >
                {sec.title}
              </Text>
            </div>
          );
        })}
      </div>
    </Flex>
  );
}

export default function GuidePage() {
  const isMobile = useIsMobile();
  const [params, setParams] = useSearchParams();

  // Timeline position: step i plays over (i, i + 1] (see stepAt). Start by
  // playing the step in the URL.
  const initial = Math.max(
    0,
    GUIDE_STEPS.findIndex((s) => s.id === params.get("step")),
  );
  const [position, setPosition] = useState(initial);
  const [view, setView] = useState<[number, number, number] | null>(null);
  const debug = useDebugTools();
  const { index } = stepAt(position, GUIDE_STEPS.length);
  const step = GUIDE_STEPS[index];
  const section = GUIDE.find((s) => s.steps.includes(step))!;

  const rafRef = useRef(0);
  const stop = useCallback(() => cancelAnimationFrame(rafRef.current), []);
  const play = useCallback(
    (i: number) => {
      stop();
      const start = performance.now();
      const tick = (now: number) => {
        const t = Math.min((now - start) / 1000 / stepSeconds(GUIDE_STEPS[i]), 1);
        setPosition(i + t);
        if (t < 1) rafRef.current = requestAnimationFrame(tick);
      };
      setPosition(i);
      rafRef.current = requestAnimationFrame(tick);
    },
    [stop],
  );
  // Back: slide this step's parts out, then (in the scene) the camera returns
  // to the previous step.
  const partsStart = useRef(0);
  const back = useCallback(() => {
    stop();
    // The parts move over [to, end] of the step; replay that backwards.
    const to = index + partsStart.current;
    const current = GUIDE_STEPS[index];
    const end = to + partsSeconds(current) / stepSeconds(current);
    const from = Math.min(position, end);
    if (from <= to) {
      setPosition(index);
      return;
    }
    const start = performance.now();
    const seconds = (from - to) * stepSeconds(current);
    const tick = (now: number) => {
      const t = Math.min((now - start) / 1000 / seconds, 1);
      if (t < 1) {
        setPosition(from + (to - from) * t);
        rafRef.current = requestAnimationFrame(tick);
      } else {
        setPosition(index);
      }
    };
    rafRef.current = requestAnimationFrame(tick);
  }, [stop, position, index]);

  useEffect(() => {
    play(initial);
    return stop;
    // Only on mount: afterwards the URL follows the timeline, not the other way.
  }, []);
  useEffect(() => {
    setParams({ step: GUIDE_STEPS[index].id }, { replace: true });
  }, [index, setParams]);

  const go = play;

  return (
    <Box className={styles.root}>
      <Suspense fallback={null}>
        <GuideScene
          steps={GUIDE_STEPS}
          position={position}
          onView={setView}
          onPartsStart={(f) => (partsStart.current = f)}
        />
      </Suspense>

      {debug && (
        <Flex direction="column" gap="1" className={styles.debug}>
          <Text size="1" color="gray" className={styles.selectable}>
            step {step.id}
          </Text>
          {view && (
            <Text size="1" color="gray" className={styles.selectable}>
              view [{view.join(", ")}]
            </Text>
          )}
          <Text size="1" color="gray">
            ? to hide
          </Text>
        </Flex>
      )}

      <div className={isMobile ? styles.overlayMobile : styles.overlay}>
        <Card size="2" className={styles.panel}>
          <Flex direction="column" gap="3">
            {index === 0 && (
              <Text size="1" color="gray">
                <MarkdownText>{GUIDE_INTRO}</MarkdownText>
              </Text>
            )}

            <Select.Root
              value={step.id}
              onValueChange={(id) =>
                go(GUIDE_STEPS.findIndex((s) => s.id === id))
              }
            >
              <Select.Trigger variant="ghost" />
              <Select.Content position="popper">
                {GUIDE.map((sec) => (
                  <Select.Group key={sec.title}>
                    <Select.Label>{sec.title}</Select.Label>
                    {sec.steps.map((s) => (
                      <Select.Item key={s.id} value={s.id}>
                        {GUIDE_STEPS.indexOf(s) + 1}. {s.title}
                      </Select.Item>
                    ))}
                  </Select.Group>
                ))}
              </Select.Content>
            </Select.Root>

            <Flex direction="column" gap="1">
              <Text size="1" color="gray">
                {section.title} · step {index + 1} of {GUIDE_STEPS.length}
              </Text>
              <Heading size="4">{step.title}</Heading>
            </Flex>

            <MarkdownText>{step.body}</MarkdownText>

            {step.instances.length > 0 && (
              <Flex direction="column" gap="2">
                <Text size="2" weight="bold">
                  Parts
                </Text>
                <StepParts instances={step.instances} />
              </Flex>
            )}

            <PartFiles instances={step.instances} className={styles.markdown} />

            <Flex justify="between" align="center" gap="2">
              <Button
                variant="soft"
                disabled={index === 0}
                onClick={back}
              >
                <ChevronLeftIcon /> Back
              </Button>
              <IconButton
                variant="ghost"
                aria-label="Replay step"
                onClick={() => play(index)}
              >
                <ReloadIcon />
              </IconButton>
              <Button
                disabled={index === GUIDE_STEPS.length - 1}
                onClick={() => go(index + 1)}
              >
                Next <ChevronRightIcon />
              </Button>
            </Flex>
          </Flex>
        </Card>

        <Card size="2" className={styles.timeline}>
          <Timeline
            position={position}
            onScrub={(v) => {
              stop();
              setPosition(v);
            }}
            onStep={play}
          />
        </Card>
      </div>
    </Box>
  );
}
