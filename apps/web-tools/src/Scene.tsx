import {
  useRef,
  useMemo,
  useCallback,
  useImperativeHandle,
  forwardRef,
  memo,
} from "react";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import {
  Environment,
  OrbitControls,
  OrthographicCamera,
} from "@react-three/drei";
import type { OrbitControls as OrbitControlsImpl } from "three-stdlib";
import { ACESFilmicToneMapping, Box3, Vector3 } from "three";
import type { Simulator } from "@ossm-rs/web-simulator";
import { useAppearance } from "./hooks/useAppearance";
import { useOssmModel } from "./model/useOssmModel";

const RAIL_TRAVEL = 0.25;
// The model has the rail at its assembled (centred) position; at full extension
// it sits this much further out.
const RAIL_EXTENDED_OFFSET = 0.14;

function Model({
  simulator,
  onOrbitTarget,
  overridePosition,
}: {
  simulator: Simulator;
  onOrbitTarget?: (center: Vector3) => void;
  overridePosition?: number | null;
}) {
  const scene = useOssmModel();

  const rail = useMemo(() => {
    const housing = scene.getObjectByName("housing");
    if (housing) {
      const center = new Vector3();
      new Box3().setFromObject(housing).getCenter(center);
      onOrbitTarget?.(center);
    }
    // The belt is modelled in its assembled shape; it can't follow the stroke.
    const belt = scene.getObjectByName("belt");
    if (belt) belt.visible = false;
    return scene.getObjectByName("rail") ?? null;
  }, [scene, onOrbitTarget]);

  useFrame(() => {
    if (!rail) return;
    const pos =
      overridePosition != null ? overridePosition : simulator.get_position();
    rail.position.z = RAIL_EXTENDED_OFFSET - (1 - pos) * RAIL_TRAVEL;
  });

  return <primitive object={scene} />;
}

export interface SceneHandle {
  resetView: () => void;
}

const INITIAL_CAMERA: [number, number, number] = [-0.4, 0.4, 0.4];
const RESET_LERP_SPEED = 0.08;
const RESET_SNAP_THRESHOLD = 0.0001;

interface ViewportInsets {
  top: number;
  left: number;
  width: number;
  height: number;
}

function SceneContent({
  simulator,
  handle,
  zoom,
  viewportInsets,
  overridePosition,
}: {
  simulator: Simulator;
  handle: React.Ref<SceneHandle>;
  zoom: number;
  viewportInsets?: ViewportInsets;
  overridePosition?: number | null;
}) {
  const [appearance] = useAppearance();
  const controlsRef = useRef<OrbitControlsImpl>(null);
  const resettingRef = useRef(false);
  const camera = useThree((s) => s.camera);
  const targetRef = useRef(new Vector3());

  const isDark = appearance === "dark";

  const goalPos = useMemo(() => new Vector3(...INITIAL_CAMERA), []);
  const goalTarget = targetRef.current;

  const onOrbitTarget = useCallback((center: Vector3) => {
    targetRef.current.copy(center);
    controlsRef.current?.target.copy(center);
    controlsRef.current?.update();
  }, []);

  useImperativeHandle(handle, () => ({
    resetView: () => {
      resettingRef.current = true;
    },
  }));

  const size = useThree((s) => s.size);

  useFrame(({ gl }) => {
    gl.toneMappingExposure = isDark ? 0.75 : 1.2;

    if (viewportInsets && viewportInsets.width > 0 && viewportInsets.height > 0) {
      const { top, left, width, height } = viewportInsets;
      // Center of the viewport sub-region vs center of the full canvas
      const vpCenterX = left + width / 2;
      const vpCenterY = top + height / 2;
      // setViewOffset's x/y is the top-left of the sub-window we want centered.
      // To shift the projection so vpCenter becomes the visual center:
      const offsetX = size.width / 2 - vpCenterX;
      const offsetY = size.height / 2 - vpCenterY;
      camera.setViewOffset(
        size.width,
        size.height,
        offsetX,
        offsetY,
        size.width,
        size.height,
      );
      camera.updateProjectionMatrix();
    } else {
      camera.clearViewOffset();
      camera.updateProjectionMatrix();
    }

    if (!resettingRef.current) return;
    const controls = controlsRef.current;
    if (!controls) return;

    camera.position.lerp(goalPos, RESET_LERP_SPEED);
    controls.target.lerp(goalTarget, RESET_LERP_SPEED);
    camera.zoom += (zoom - camera.zoom) * RESET_LERP_SPEED;
    camera.updateProjectionMatrix();
    controls.update();

    if (
      camera.position.distanceTo(goalPos) < RESET_SNAP_THRESHOLD &&
      controls.target.distanceTo(goalTarget) < RESET_SNAP_THRESHOLD &&
      Math.abs(camera.zoom - zoom) < 0.5
    ) {
      camera.position.copy(goalPos);
      controls.target.copy(goalTarget);
      camera.zoom = zoom;
      camera.updateProjectionMatrix();
      controls.update();
      resettingRef.current = false;
    }
  });

  return (
    <>
      <color attach="background" args={[isDark ? "#111113" : "#ffffff"]} />
      <ambientLight intensity={isDark ? 0.4 : 0.8} />
      <directionalLight position={[1, 2, 3]} intensity={isDark ? 0.8 : 1.5} />
      <directionalLight position={[-1, 1, -1]} intensity={isDark ? 0.3 : 0.5} />
      <Environment preset="studio" environmentIntensity={isDark ? 0.4 : 1} />
      <Model simulator={simulator} onOrbitTarget={onOrbitTarget} overridePosition={overridePosition} />
      <OrthographicCamera
        makeDefault
        position={INITIAL_CAMERA}
        zoom={zoom}
        near={0.001}
        far={10}
      />
      <OrbitControls ref={controlsRef} />
    </>
  );
}

const Scene = memo(
  forwardRef<
    SceneHandle,
    {
      simulator: Simulator;
      zoom?: number;
      viewportInsets?: ViewportInsets;
      overridePosition?: number | null;
    }
  >(function Scene({ simulator, zoom = 1500, viewportInsets, overridePosition }, ref) {
    return (
      <Canvas
        gl={{ toneMapping: ACESFilmicToneMapping, toneMappingExposure: 1.2 }}
        style={{ width: "100%", height: "100%" }}
      >
        <SceneContent
          simulator={simulator}
          handle={ref}
          zoom={zoom}
          viewportInsets={viewportInsets}
          overridePosition={overridePosition}
        />
      </Canvas>
    );
  }),
);

export default Scene;
