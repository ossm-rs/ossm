import { useEffect, useMemo, useRef } from "react";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import {
  Environment,
  GizmoHelper,
  GizmoViewcube,
  OrbitControls,
  OrthographicCamera,
} from "@react-three/drei";
import type { OrbitControls as OrbitControlsImpl } from "three-stdlib";
import {
  ACESFilmicToneMapping,
  Box3,
  Mesh,
  MeshBasicMaterial,
  Vector3,
} from "three";
import type { Material, Object3D } from "three";
import { useAppearance } from "./hooks/useAppearance";
import { useDebugTools } from "./hooks/useDebugTools";
import { useOssmModel } from "./model/useOssmModel";
import { PART_CATALOG } from "./parts";
import type { GuideStep } from "./guide/ossm-alt";
import type { Belt, BeltPose } from "./model/belt";

const INITIAL_CAMERA = new Vector3(0.4, 0.4, 0.4);
// Each step moves the camera, pauses, fades its new parts in where they start,
// pauses again, then slides the parts in. A step whose framing barely changes
// skips straight to the fade.
const CAMERA_SECONDS = 0.9;
const PAUSE_SECONDS = 0.5;
const FADE_SECONDS = 0.4;
const HOLD_SECONDS = 0.5;
const SLIDE_SECONDS = 0.9;

/** How long a step's parts take (fade, hold, then slide; a step may set its slide). */
export function partsSeconds(step: GuideStep): number {
  return FADE_SECONDS + HOLD_SECONDS + (step.slide ?? SLIDE_SECONDS);
}

/** How long a step plays for. */
export function stepSeconds(step: GuideStep): number {
  return CAMERA_SECONDS + PAUSE_SECONDS + partsSeconds(step);
}
// The view frames this step's parts with some of the build around them:
// small parts get room around them, long ones (the belt) a fixed margin.
const CONTEXT = 2.5;
const MAX_MARGIN = 0.1;
const MIN_EXTENT = 0.12;
// Parts the catalog doesn't move slide in from above.
const DEFAULT_FROM = new Vector3(0, 0.05, 0);

/** Where each node slides in from by default: its hand-tuned explode offset
 *  in the catalog (a step's `from` overrides it). */
function slideOffsets(): Map<string, Vector3> {
  const out = new Map<string, Vector3>();
  for (const part of PART_CATALOG) {
    part.instances.forEach((name, i) => {
      const own = part.offsets ? part.offsets[i] : part.offset;
      if (own && (own[0] || own[1] || own[2])) {
        out.set(name, new Vector3(...own));
      }
    });
  }
  return out;
}

type GuideNode = {
  node: Object3D;
  home: Vector3;
  /** Slide-in offset when the node's step plays. */
  from: Vector3;
  /** Step that adds it, and (for staged parts) the step that places it. */
  step: number;
  placed: number;
  staged: Vector3 | null;
  /** Its own copies of the shared materials, so it can fade alone. */
  materials: Material[];
  /** Depth-only copies of its meshes, on while it fades (see fadeable). */
  prepass: Mesh[];
  opacity: number;
};

function setOpacity(n: GuideNode, opacity: number) {
  if (n.opacity === opacity) return;
  n.opacity = opacity;
  const fading = opacity < 1;
  for (const m of n.materials) {
    // Opaque once in, so it sorts and depth-tests like everything else.
    m.transparent = fading;
    m.depthWrite = !fading;
    m.opacity = opacity;
  }
  for (const p of n.prepass) p.visible = fading;
}

// Writes depth only. Drawn after the opaque parts (renderOrder) so whatever is
// behind a fading part still shows through it.
const PREPASS = new MeshBasicMaterial({ colorWrite: false });

/** Gives a node its own materials, and a depth pre-pass so that while it
 *  fades only its nearest surface blends in, not its inside walls. */
function fadeable(node: Object3D): { materials: Material[]; prepass: Mesh[] } {
  const meshes: Mesh[] = [];
  node.traverse((o) => {
    const mesh = o as Mesh;
    if (mesh.isMesh && !Array.isArray(mesh.material)) meshes.push(mesh);
  });
  const materials: Material[] = [];
  const prepass: Mesh[] = [];
  for (const mesh of meshes) {
    mesh.material = (mesh.material as Material).clone();
    materials.push(mesh.material);
    const depth = new Mesh(mesh.geometry, PREPASS);
    depth.renderOrder = 1;
    depth.visible = false;
    mesh.add(depth);
    prepass.push(depth);
  }
  return { materials, prepass };
}

/** How the belt lies at `stepIndex`, `slide` (0..1) into its slide phase. */
function beltPose(steps: GuideStep[], stepIndex: number, slide: number): BeltPose {
  for (let i = stepIndex; i >= 0; i--) {
    const b = steps[i].belt;
    if (!b) continue;
    if (b.mode === "routed") return b;
    // A step's own belt move plays with its slide; later steps hold its end.
    const t = i === stepIndex ? easeOutCubic(slide) : 1;
    const mix = ([from, to]: [number, number]) => from + (to - from) * t;
    if (b.mode === "thread") return { mode: "thread", progress: t };
    if (b.mode === "rear") return { mode: "rear", feed: mix(b.feed), slack: mix(b.slack) };
    return { mode: "hang", end: b.end, feed: mix(b.feed), approach: b.approach };
  }
  return { mode: "routed" };
}

/** Where a node sits once `stepIndex` has finished playing. */
function restingPlace(n: GuideNode, stepIndex: number): Vector3 {
  const p = n.home.clone();
  if (n.staged && stepIndex < n.placed) p.add(n.staged);
  return p;
}

function easeOutCubic(t: number) {
  return 1 - Math.pow(1 - t, 3);
}

function easeInOutCubic(t: number) {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

const clamp01 = (t: number) => Math.min(Math.max(t, 0), 1);

/** The camera move at the start of a step, from wherever the camera was. */
type CameraMove = {
  /** Set when the move runs on the clock rather than the timeline: the step
   *  was entered past its camera phase (going back, or scrubbing). */
  startedAt: number | null;
  fromTarget: Vector3;
  toTarget: Vector3;
  fromDir: Vector3;
  toDir: Vector3;
  distance: number;
  fromZoom: number;
  toZoom: number;
};

/** Which step `position` is in, and how far its parts have slid in (0..1).
 *  Step i plays over (i, i + 1]; at i + 1 it's complete. */
export function stepAt(position: number, count: number) {
  const index = Math.min(Math.max(Math.ceil(position) - 1, 0), count - 1);
  return { index, progress: Math.min(Math.max(position - index, 0), 1) };
}

// View cube faces in BoxGeometry order (+X, -X, +Y, -Y, +Z, -Z), named after
// the machine so orientations can be described in its terms.
const CUBE_FACES = ["PCB", "Belt", "Top", "Bottom", "Toy end", "Rear end"];

/** Where the camera looks from, rounded like a step's `view`. */
function viewOf(camera: Object3D, target: Vector3): [number, number, number] {
  const d = camera.position.clone().sub(target).normalize();
  const r = (x: number) => Math.round(x * 100) / 100 || 0;
  return [r(d.x), r(d.y), r(d.z)];
}

function Model({
  steps,
  position,
  onView,
  onPartsStart,
}: {
  steps: GuideStep[];
  position: number;
  onView?: (view: [number, number, number]) => void;
  onPartsStart?: (fraction: number) => void;
}) {
  const { index: stepIndex, progress } = stepAt(position, steps.length);
  const scene = useOssmModel();
  const camera = useThree((s) => s.camera);
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null;
  const size = useThree((s) => s.size);

  const belt = scene.userData.belt as Belt | null;
  const nodes = useMemo(() => {
    const from = slideOffsets();
    const out = new Map<string, GuideNode>();
    steps.forEach((step, k) =>
      step.instances.forEach((name) => {
        const node = scene.getObjectByName(name);
        if (!node) {
          console.warn(`[guide] node not found: ${name} (step ${step.id})`);
          return;
        }
        out.set(name, {
          node,
          home: node.position.clone(),
          // A step's main part appears in place; the rest travel to it. The belt
          // doesn't slide: its pose moves it (see beltPose).
          from:
            step.main === name || node === belt?.mesh
              ? new Vector3()
              : step.from?.[name]
                ? new Vector3(...step.from[name])
                : (from.get(name) ?? DEFAULT_FROM),
          step: k,
          staged: step.stage ? new Vector3(...step.stage) : null,
          placed: Infinity,
          ...fadeable(node),
          opacity: 1,
        });
      }),
    );
    steps.forEach((step, k) =>
      step.place?.forEach((name) => {
        const n = out.get(name);
        if (!n?.staged || n.step >= k) {
          console.warn(`[guide] ${step.id} places ${name}, which no earlier step staged`);
          return;
        }
        n.placed = k;
      }),
    );
    return out;
  }, [scene, steps, belt]);

  // Compile the fading variants of every material now: switching a material to
  // transparent needs another shader, and compiling it mid-fade stalls the
  // frame (badly in Firefox), so the part blips in.
  const gl = useThree((s) => s.gl);
  const root = useThree((s) => s.scene);
  useEffect(() => {
    for (const n of nodes.values()) {
      n.node.visible = true;
      setOpacity(n, 0.5);
    }
    gl.compile(root, camera);
    // useFrame puts visibility and opacity back on the next frame.
    for (const n of nodes.values()) setOpacity(n, 1);
  }, [gl, root, camera, nodes]);

  const goalTarget = useRef(new Vector3());
  const goalView = useRef<Vector3 | null>(null);
  const goalZoom = useRef(1500);
  // A new framing to move to, picked up by the next frame.
  const framing = useRef(false);
  const firstFrame = useRef(true);
  const move = useRef<CameraMove | null>(null);
  // Whether this step moves the camera before its parts.
  const cameraMoves = useRef(false);

  // On a new step: centre the camera on its main part, and zoom to take in
  // that part and whatever the step adds or places, where they end up.
  useEffect(() => {
    const boxAt = (n: GuideNode) => {
      const moved = n.node.position.clone();
      n.node.position.copy(restingPlace(n, stepIndex));
      n.node.updateWorldMatrix(true, true);
      const b = new Box3().expandByObject(n.node);
      n.node.position.copy(moved);
      return b;
    };
    const all = steps[stepIndex].frame === "all";
    const main = nodes.get(steps[stepIndex].main);
    const box = main ? boxAt(main) : new Box3();
    for (const n of nodes.values()) {
      // Not the belt: hanging, it would pull the view far out.
      if (n.node === belt?.mesh) continue;
      const shown = all ? n.step <= stepIndex : n.step === stepIndex || n.placed === stepIndex;
      if (shown) box.union(boxAt(n));
    }
    // Centred on the main part, or on everything when framing it all.
    if (all || !main) box.getCenter(goalTarget.current);
    else boxAt(main).getCenter(goalTarget.current);
    if (box.isEmpty()) return;
    // Views carry forward: the latest one set at or before this step.
    const view = steps
      .slice(0, stepIndex + 1)
      .reverse()
      .find((st) => st.view)?.view;
    goalView.current = view ? new Vector3(...view).normalize() : null;
    // The main part is centred, so size the view by the farthest corner from it.
    const corners = [box.min, box.max].flatMap((a) =>
      [box.min, box.max].flatMap((b) =>
        [box.min, box.max].map((c) => new Vector3(a.x, b.y, c.z)),
      ),
    );
    const diagonal =
      2 * Math.max(...corners.map((c) => c.distanceTo(goalTarget.current))) * 0.7;
    const extent = Math.max(
      Math.min(diagonal * CONTEXT, diagonal + MAX_MARGIN),
      MIN_EXTENT,
    );
    goalZoom.current = Math.min(size.width, size.height) / extent;
    framing.current = true;
  }, [nodes, steps, stepIndex, size.width, size.height, belt]);

  const lastView = useRef("");
  useFrame(() => {
    if (controls && onView) {
      const v = viewOf(camera, controls.target);
      const key = v.join(",");
      if (key !== lastView.current) {
        lastView.current = key;
        onView(v);
      }
    }
    if (framing.current && controls) {
      framing.current = false;
      const offset = camera.position.clone().sub(controls.target);
      const dir = offset.clone().normalize();
      const toDir = goalView.current ?? dir;
      if (firstFrame.current) {
        // Opening the guide: start at the step's framing rather than moving to it.
        firstFrame.current = false;
        controls.target.copy(goalTarget.current);
        camera.position.copy(goalTarget.current).addScaledVector(toDir, offset.length());
        camera.zoom = goalZoom.current;
        camera.updateProjectionMatrix();
        controls.update();
        cameraMoves.current = false;
        onPartsStart?.(0);
      } else {
        const still =
          controls.target.distanceTo(goalTarget.current) < 0.001 &&
          dir.angleTo(toDir) < 0.02 &&
          Math.abs(Math.log(goalZoom.current / camera.zoom)) < 0.05;
        cameraMoves.current = !still;
        onPartsStart?.(still ? 0 : (CAMERA_SECONDS + PAUSE_SECONDS) / stepSeconds(steps[stepIndex]));
        move.current = {
          startedAt:
            progress * stepSeconds(steps[stepIndex]) >= CAMERA_SECONDS ? performance.now() : null,
          fromTarget: controls.target.clone(),
          toTarget: goalTarget.current.clone(),
          fromDir: dir,
          toDir: toDir.clone(),
          distance: offset.length(),
          fromZoom: camera.zoom,
          toZoom: goalZoom.current,
        };
      }
    }
    // Where we are in this step, in seconds of its playback.
    const seconds = progress * stepSeconds(steps[stepIndex]);
    const m = move.current;
    if (m && controls) {
      const t =
        m.startedAt !== null
          ? clamp01((performance.now() - m.startedAt) / 1000 / CAMERA_SECONDS)
          : cameraMoves.current
            ? clamp01(seconds / CAMERA_SECONDS)
            : 1;
      const e = easeInOutCubic(t);
      controls.target.lerpVectors(m.fromTarget, m.toTarget, e);
      const dir = m.fromDir.clone().lerp(m.toDir, e).normalize();
      camera.position.copy(controls.target).addScaledVector(dir, m.distance);
      camera.zoom = m.fromZoom * Math.pow(m.toZoom / m.fromZoom, e);
      camera.updateProjectionMatrix();
      controls.update();
      // Done: the camera is the viewer's again.
      if (t >= 1) move.current = null;
    }

    // Then the parts: new ones fade in where they start, hold, then everything slides.
    const partsStart = cameraMoves.current ? CAMERA_SECONDS + PAUSE_SECONDS : 0;
    const fade = clamp01((seconds - partsStart) / FADE_SECONDS);
    const slide = clamp01(
      (seconds - partsStart - FADE_SECONDS - HOLD_SECONDS) /
        (steps[stepIndex].slide ?? SLIDE_SECONDS),
    );
    const k = 1 - easeOutCubic(slide);
    for (const n of nodes.values()) {
      const fresh = n.step === stepIndex;
      n.node.visible = n.step < stepIndex || (fresh && fade > 0);
      setOpacity(n, fresh ? fade : 1);
      n.node.position.copy(restingPlace(n, stepIndex));
      if (n.step === stepIndex) {
        n.node.position.addScaledVector(n.from, k); // slides in
      } else if (n.placed === stepIndex && n.staged) {
        n.node.position.addScaledVector(n.staged, k); // moves from its staging spot
      }
    }
    if (belt) {
      belt.setPose(beltPose(steps, stepIndex, slide));
      belt.update();
    }
  });

  return <primitive object={scene} />;
}

type SceneProps = {
  steps: GuideStep[];
  position: number;
  /** Called with the camera's viewing direction whenever it changes. */
  onView?: (view: [number, number, number]) => void;
  /** Called on entering a step with where (0..1) its parts start moving. */
  onPartsStart?: (fraction: number) => void;
};

function SceneContent(props: SceneProps) {
  const [appearance] = useAppearance();
  const debug = useDebugTools();
  const isDark = appearance === "dark";

  useFrame(({ gl }) => {
    gl.toneMappingExposure = isDark ? 0.75 : 1.2;
  });

  return (
    <>
      <color attach="background" args={[isDark ? "#111113" : "#ffffff"]} />
      <ambientLight intensity={isDark ? 0.4 : 0.8} />
      <directionalLight position={[1, 2, 3]} intensity={isDark ? 0.8 : 1.5} />
      <directionalLight position={[-1, 1, -1]} intensity={isDark ? 0.3 : 0.5} />
      <Environment preset="studio" environmentIntensity={isDark ? 0.4 : 1} />
      <Model {...props} />
      <OrthographicCamera
        makeDefault
        position={INITIAL_CAMERA}
        zoom={1500}
        near={0.001}
        far={10}
      />
      <OrbitControls makeDefault />
      {debug && (
        <GizmoHelper alignment="top-right" margin={[84, 84]}>
          <group scale={1.35}>
            <GizmoViewcube
              faces={CUBE_FACES}
              font="bold 20px Inter, system-ui, sans-serif"
              color={isDark ? "#2b2b30" : "#ffffff"}
              hoverColor={isDark ? "#6e3fa3" : "#d8c6f0"}
              textColor={isDark ? "#f4f4f5" : "#27272a"}
              strokeColor={isDark ? "#71717a" : "#71717a"}
            />
          </group>
        </GizmoHelper>
      )}
    </>
  );
}

/** The build at `position` along the guide (see stepAt). */
export default function GuideScene(props: SceneProps) {
  return (
    <Canvas
      gl={{ toneMapping: ACESFilmicToneMapping, toneMappingExposure: 1.2 }}
      style={{ width: "100%", height: "100%" }}
    >
      <SceneContent {...props} />
    </Canvas>
  );
}
