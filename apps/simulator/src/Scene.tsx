import { useRef, useMemo, useImperativeHandle, forwardRef, memo } from "react";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { Environment, OrbitControls, useGLTF } from "@react-three/drei";
import type { OrbitControls as OrbitControlsImpl } from "three-stdlib";
import { ACESFilmicToneMapping, Vector3 } from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import { MeshStandardMaterial, Mesh as ThreeMesh, BufferGeometry } from "three";
import type { Simulator } from "sim-wasm";
import type { Object3D } from "three";
import { useAppearance } from "./hooks/useAppearance";

const MODEL_URL = "/models/ossm-alt.gltf";

const TRAVEL_M = 0.25;

const purpleMaterial = new MeshStandardMaterial({ color: 0x6a1b9a });
const railMaterial = new MeshStandardMaterial({
  color: 0x888888,
  metalness: 0.8,
  roughness: 0.2,
});

function collectGeometries(root: Object3D): BufferGeometry | null {
  const geometries: BufferGeometry[] = [];
  root.updateWorldMatrix(true, true);
  root.traverse((child) => {
    if ((child as ThreeMesh).isMesh) {
      const mesh = child as ThreeMesh;
      const geo = mesh.geometry.clone();
      geo.applyMatrix4(mesh.matrixWorld);
      geometries.push(geo);
    }
  });
  if (geometries.length === 0) return null;
  return mergeGeometries(geometries);
}

function Model({ simulator }: { simulator: Simulator }) {
  const { scene } = useGLTF(MODEL_URL);
  const railRef = useRef<ThreeMesh>(null);

  const { housingGeo, railGeo } = useMemo(() => {
    const housingNode = scene.getObjectByName("housing");
    const railNode = scene.getObjectByName("rail");
    return {
      housingGeo: housingNode ? collectGeometries(housingNode) : null,
      railGeo: railNode ? collectGeometries(railNode) : null,
    };
  }, [scene]);

  useFrame(() => {
    if (railRef.current) {
      const pos = simulator.get_position();
      railRef.current.position.z = -(1 - pos) * TRAVEL_M;
    }
  });

  return (
    <>
      {housingGeo && <mesh geometry={housingGeo} material={purpleMaterial} />}
      {railGeo && (
        <mesh ref={railRef} geometry={railGeo} material={railMaterial} />
      )}
    </>
  );
}

export interface SceneHandle {
  resetView: () => void;
}

const INITIAL_CAMERA: [number, number, number] = [-0.373, 0.2624, 0.458];
const INITIAL_TARGET: [number, number, number] = [0, 0.03, 0.05];

function SceneContent({
  simulator,
  handle,
}: {
  simulator: Simulator;
  handle: React.Ref<SceneHandle>;
}) {
  const [appearance] = useAppearance();
  const controlsRef = useRef<OrbitControlsImpl>(null);
  const resettingRef = useRef(false);
  const camera = useThree((s) => s.camera);

  const isDark = appearance === "dark";

  const goalPos = useMemo(() => new Vector3(...INITIAL_CAMERA), []);
  const goalTarget = useMemo(() => new Vector3(...INITIAL_TARGET), []);

  useImperativeHandle(handle, () => ({
    resetView: () => {
      resettingRef.current = true;
    },
  }));

  useFrame(({ gl }) => {
    gl.toneMappingExposure = isDark ? 0.75 : 1.2;

    if (!resettingRef.current) return;
    const controls = controlsRef.current;
    if (!controls) return;

    const alpha = 0.08;
    camera.position.lerp(goalPos, alpha);
    controls.target.lerp(goalTarget, alpha);
    controls.update();

    if (
      camera.position.distanceTo(goalPos) < 0.0001 &&
      controls.target.distanceTo(goalTarget) < 0.0001
    ) {
      camera.position.copy(goalPos);
      controls.target.copy(goalTarget);
      controls.update();
      resettingRef.current = false;
    }
  });

  return (
    <>
      <color
        attach="background"
        args={[isDark ? "#111113" : "#ffffff"]}
      />
      <ambientLight intensity={isDark ? 0.4 : 0.8} />
      <directionalLight
        position={[1, 2, 3]}
        intensity={isDark ? 0.8 : 1.5}
      />
      <directionalLight
        position={[-1, 1, -1]}
        intensity={isDark ? 0.3 : 0.5}
      />
      <Environment preset="studio" environmentIntensity={isDark ? 0.4 : 1} />
      <Model simulator={simulator} />
      <OrbitControls ref={controlsRef} target={INITIAL_TARGET} />
    </>
  );
}

const Scene = memo(forwardRef<
  SceneHandle,
  { simulator: Simulator }
>(function Scene({ simulator }, ref) {
  return (
    <Canvas
      camera={{
        position: INITIAL_CAMERA,
        fov: 45,
        near: 0.001,
        far: 10,
      }}
      gl={{ toneMapping: ACESFilmicToneMapping, toneMappingExposure: 1.2 }}
      style={{ width: "100%", height: "100%" }}
    >
      <SceneContent
        simulator={simulator}
        handle={ref}
      />
    </Canvas>
  );
}));

export default Scene;
