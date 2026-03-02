import { useRef, useMemo } from "react";
import { Canvas, useFrame } from "@react-three/fiber";
import { Environment, OrbitControls, useGLTF } from "@react-three/drei";
import { ACESFilmicToneMapping } from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import { MeshStandardMaterial, Mesh as ThreeMesh, BufferGeometry } from "three";
import type { Simulator } from "sim-wasm";
import type { Object3D } from "three";

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

export default function Scene({ simulator }: { simulator: Simulator }) {
  return (
    <Canvas
      camera={{
        position: [-0.373, 0.2624, 0.458],
        fov: 45,
        near: 0.001,
        far: 10,
      }}
      gl={{ toneMapping: ACESFilmicToneMapping, toneMappingExposure: 1.2 }}
      style={{ width: "100%", height: "100%" }}
    >
      <color attach="background" args={["#ffffff"]} />
      <ambientLight intensity={0.8} />
      <directionalLight position={[1, 2, 3]} intensity={1.5} />
      <directionalLight position={[-1, 1, -1]} intensity={0.5} />
      <Environment preset="studio" />
      <Model simulator={simulator} />
      <OrbitControls target={[0, 0.03, 0.05]} />
    </Canvas>
  );
}
