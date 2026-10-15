import { useRef } from "react";
import { useConfigureContext, useFrame, useRootWithStatus } from "@typegpu/react";
import {
  VERTEX_COUNT,
  flowerParams,
  fragment,
  liveSpin,
  params,
  vertex,
  type Scene,
} from "./flower.sts";

export function Canvas(props: Scene & { speed: number }) {
  const root = useRootWithStatus();
  if (root.status === "rejected") {
    const reason = root.reason;
    const message = reason instanceof Error ? reason.message : String(reason);
    return <p className="stage-note">{message}</p>;
  }
  if (root.status !== "fulfilled") return <canvas />;
  return <Drawing gpu={root.value} {...props} />;
}

function Drawing(props: Scene & { speed: number; gpu: any }) {
  const { ref, ctxRef } = useConfigureContext();
  const latest = useRef(props);
  latest.current = props;
  const clock = useRef(0);
  const pipeline = useRef<any>(null);
  const uniform = useRef<any>(null);
  useFrame(({ deltaSeconds }) => {
    const view = ctxRef.current;
    const gpu = latest.current.gpu;
    if (!view || !gpu) return;
    clock.current += deltaSeconds;
    const tint = latest.current.tint;
    const value = {
      scale: latest.current.scale,
      petals: latest.current.petals,
      point: latest.current.point,
      spin: liveSpin(0, clock.current, latest.current.speed),
      time: clock.current,
      tint: [tint[0], tint[1], tint[2]],
    };
    if (uniform.current === null) {
      uniform.current = gpu.createUniform(flowerParams, value);
      pipeline.current = gpu.with(params, uniform.current).createRenderPipeline({ vertex, fragment });
    } else {
      uniform.current.write(value);
    }
    pipeline.current
      .withColorAttachment({
        view,
        clearValue: [0.97, 0.95, 0.91, 1],
        loadOp: "clear",
        storeOp: "store",
      })
      .draw(VERTEX_COUNT);
  });
  return <canvas ref={ref} />;
}
