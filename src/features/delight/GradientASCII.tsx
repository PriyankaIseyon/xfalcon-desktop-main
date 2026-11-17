import { useEffect, useRef } from "react";
import "./GradientASCII.css";

const VERTEX_SHADER = `#version 300 es
in vec2 aPosition;
out vec2 vUv;
void main() {
  vUv = aPosition * 0.5 + 0.5;
  gl_Position = vec4(aPosition, 0.0, 1.0);
}`;

const FRAGMENT_SHADER = `#version 300 es
precision highp float;
uniform sampler2D uAtlas;
uniform vec2 uResolution;
uniform vec2 uCell;
uniform float uTime;
uniform float uReducedMotion;
uniform vec2 uPointer;
uniform float uPointerActive;
uniform float uClickTime;
uniform float uClickActive;
in vec2 vUv;
out vec4 outColor;

float hash(vec2 p) {
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float glyph(float index, vec2 local) {
  float columns = 10.0;
  float column = mod(index, columns);
  float row = floor(index / columns);
  // Canvas rows begin at the top; WebGL texture coordinates begin at the bottom.
  vec2 atlasUv = vec2((column + local.x) / columns, (row + 1.0 - local.y) / 1.0);
  return texture(uAtlas, atlasUv).a;
}

void main() {
  vec2 px = gl_FragCoord.xy;
  vec2 cell = floor(px / uCell);
  vec2 local = fract(px / uCell);
  vec2 p = (vUv - 0.5) * vec2(uResolution.x / uResolution.y, 1.0);
  float t = uReducedMotion > 0.5 ? 0.0 : uTime;
  float x = p.x / max(0.5 * uResolution.x / uResolution.y, 0.5);
  float horizon = -0.17 - 0.2 * x * x + 0.02 * sin(t * 0.6);
  float edge = p.y - horizon;
  float rim = 0.38 * smoothstep(-0.05, 0.42, edge);
  float distanceToSun = length((p - vec2(0.08 * sin(t * 0.31), 0.58)) / vec2(0.95, 0.75));
  float sky = mix(0.55, 1.0, smoothstep(0.2, 0.95, distanceToSun));
  float horizonShape = smoothstep(-0.25, 0.18, edge);
  float light = clamp(mix(rim, sky, 0.22) * (0.65 + horizonShape * 0.35), 0.0, 1.0);

  float pointerDist = length((p - uPointer) * vec2(1.0, 1.45));
  float pointerGlow = uPointerActive * exp(-pointerDist * 7.0) * 0.24;
  float clickRadius = max(0.0, uClickTime) * 0.75;
  float clickRing = uClickActive * exp(-pow((pointerDist - clickRadius) / 0.06, 2.0)) * exp(-uClickTime * 1.15);
  light = clamp(light + pointerGlow + clickRing * 0.55, 0.0, 1.0);

  vec3 deepBlue = vec3(0.118, 0.247, 0.847);
  vec3 iceWhite = vec3(0.965, 0.973, 1.0);
  vec3 cyan = vec3(0.647, 0.847, 1.0);
  vec3 blue = vec3(0.220, 0.741, 0.973);
  vec3 color = mix(deepBlue, iceWhite, smoothstep(0.05, 0.98, light));
  color = mix(color, cyan, smoothstep(-0.01, 0.09, edge) * 0.65);
  color = mix(color, blue, smoothstep(-0.12, 0.08, edge) * 0.28);

  float cellRand = hash(cell);
  float flicker = uReducedMotion > 0.5 ? 0.0 : (hash(cell + floor(t * 9.0)) - 0.5) * 0.08;
  float tone = clamp(pow(light, 0.75) + flicker + pointerGlow * 0.4 + clickRing * 0.3, 0.0, 1.0);
  float index = floor(tone * 9.0 + 0.5);
  float coverage = glyph(index, local);
  float present = step(cellRand, 0.8 * mix(1.0, 0.3 + tone * 1.1, 0.55));
  float alpha = coverage * present * smoothstep(0.03, 0.1, light) * 0.85;
  vec3 ink = mix(color * 0.72, iceWhite, smoothstep(0.55, 1.0, tone));
  outColor = vec4(mix(color * 0.9, ink, alpha), 1.0);
}`;

const CHARACTERS = " =+xX0S8#@";
const CELL_SIZE = 18;
const MAX_PIXEL_RATIO = 1.5;

type ProgramInfo = {
  program: WebGLProgram;
  uniforms: {
    atlas: WebGLUniformLocation;
    resolution: WebGLUniformLocation;
    cell: WebGLUniformLocation;
    time: WebGLUniformLocation;
    reducedMotion: WebGLUniformLocation;
    pointer: WebGLUniformLocation;
    pointerActive: WebGLUniformLocation;
    clickTime: WebGLUniformLocation;
    clickActive: WebGLUniformLocation;
  };
};

type Point = { x: number; y: number };

function compileShader(gl: WebGL2RenderingContext, kind: number, source: string): WebGLShader {
  const shader = gl.createShader(kind);
  if (!shader) throw new Error("Unable to allocate ASCII shader");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const message = gl.getShaderInfoLog(shader) ?? "ASCII shader compilation failed";
    gl.deleteShader(shader);
    throw new Error(message);
  }
  return shader;
}

function createProgram(gl: WebGL2RenderingContext): ProgramInfo {
  const vertex = compileShader(gl, gl.VERTEX_SHADER, VERTEX_SHADER);
  let fragment: WebGLShader | null = null;
  let program: WebGLProgram | null = null;
  try {
    fragment = compileShader(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER);
    program = gl.createProgram();
    if (!program) throw new Error("Unable to allocate ASCII program");
    gl.attachShader(program, vertex);
    gl.attachShader(program, fragment);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      throw new Error(gl.getProgramInfoLog(program) ?? "ASCII program linking failed");
    }
    const linkedProgram = program;
    const required = (name: string) => {
      const location = gl.getUniformLocation(linkedProgram, name);
      if (!location) throw new Error(`Missing ASCII shader uniform: ${name}`);
      return location;
    };
    return {
      program,
      uniforms: {
        atlas: required("uAtlas"),
        resolution: required("uResolution"),
        cell: required("uCell"),
        time: required("uTime"),
        reducedMotion: required("uReducedMotion"),
        pointer: required("uPointer"),
        pointerActive: required("uPointerActive"),
        clickTime: required("uClickTime"),
        clickActive: required("uClickActive"),
      },
    };
  } catch (error) {
    if (program) gl.deleteProgram(program);
    throw error;
  } finally {
    gl.deleteShader(vertex);
    if (fragment) gl.deleteShader(fragment);
  }
}

function createAtlas(gl: WebGL2RenderingContext): WebGLTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 600;
  canvas.height = 100;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("Unable to create ASCII glyph atlas");
  context.clearRect(0, 0, canvas.width, canvas.height);
  context.font = '300 78px "Geist Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
  context.textAlign = "center";
  context.textBaseline = "middle";
  context.fillStyle = "#fff";
  [...CHARACTERS].forEach((character, index) => {
    context.fillText(character, index * 60 + 30, 52);
  });

  const texture = gl.createTexture();
  if (!texture) throw new Error("Unable to allocate ASCII glyph texture");
  try {
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, canvas);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    if (gl.getError() !== gl.NO_ERROR) throw new Error("Unable to upload ASCII glyph atlas");
    return texture;
  } catch (error) {
    gl.deleteTexture(texture);
    throw error;
  }
}

export function GradientASCII({ className }: { className?: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas) return;

    let gl: WebGL2RenderingContext | null = null;
    let programInfo: ProgramInfo | null = null;
    let atlas: WebGLTexture | null = null;
    let vao: WebGLVertexArrayObject | null = null;
    let buffer: WebGLBuffer | null = null;
    let frame = 0;
    let disposed = false;
    let active = false;
    let contextLost = false;
    let visible = true;
    const motionQuery = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    let reduceMotion = motionQuery?.matches ?? false;
    let lastFrame = 0;
    let lastDraw = 0;
    let time = 0;
    let pointer: Point = { x: 0, y: 0 };
    let pointerAt = -Infinity;
    let clickAt = -Infinity;
    let fontReady = false;
    let failRenderer = () => {};

    const stopFrame = () => {
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      lastFrame = 0;
      lastDraw = 0;
    };

    const fallback = () => {
      container.dataset.asciiReady = "false";
      stopFrame();
    };

    const onContextLost = (event: Event) => {
      event.preventDefault();
      contextLost = true;
      failRenderer();
    };

    canvas.addEventListener("webglcontextlost", onContextLost);

    const cleanupResources = () => {
      if (gl && !contextLost) {
        if (atlas) gl.deleteTexture(atlas);
        if (vao) gl.deleteVertexArray(vao);
        if (buffer) gl.deleteBuffer(buffer);
        if (programInfo) gl.deleteProgram(programInfo.program);
      }
      atlas = null;
      vao = null;
      buffer = null;
      programInfo = null;
      gl = null;
    };

    const refreshAtlas = () => {
      if (!fontReady || !active || disposed || contextLost || !visible || document.hidden || !gl) return;
      const renderer = gl;
      const refreshedAtlas = createAtlas(renderer);
      if (!active || disposed || contextLost || gl !== renderer) {
        renderer.deleteTexture(refreshedAtlas);
        return;
      }
      if (atlas) renderer.deleteTexture(atlas);
      atlas = refreshedAtlas;
      fontReady = false;
    };

    const draw = (now: number) => {
      if (!active || disposed || !visible || document.hidden || !gl || !programInfo || !vao || !atlas || contextLost)
        return;
      const rect = container.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      refreshAtlas();
      const ratio = Math.min(window.devicePixelRatio || 1, MAX_PIXEL_RATIO);
      const width = Math.max(1, Math.round(rect.width * ratio));
      const height = Math.max(1, Math.round(rect.height * ratio));
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      gl.viewport(0, 0, width, height);
      gl.useProgram(programInfo.program);
      gl.bindVertexArray(vao);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, atlas);
      gl.uniform1i(programInfo.uniforms.atlas, 0);
      gl.uniform2f(programInfo.uniforms.resolution, width, height);
      gl.uniform2f(programInfo.uniforms.cell, CELL_SIZE * ratio * 0.6, CELL_SIZE * ratio);
      gl.uniform1f(programInfo.uniforms.time, time);
      gl.uniform1f(programInfo.uniforms.reducedMotion, reduceMotion ? 1 : 0);
      gl.uniform2f(programInfo.uniforms.pointer, pointer.x, pointer.y);
      gl.uniform1f(programInfo.uniforms.pointerActive, now - pointerAt < 90 ? 1 : 0);
      gl.uniform1f(programInfo.uniforms.clickTime, Math.max(0, (now - clickAt) / 1000));
      gl.uniform1f(programInfo.uniforms.clickActive, now - clickAt < 1200 ? 1 : 0);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      if (gl.getError() !== gl.NO_ERROR) throw new Error("ASCII renderer reported a WebGL error");
      container.dataset.asciiReady = "true";
    };

    const animate = (now: number) => {
      frame = 0;
      if (!active || disposed || contextLost || !visible || document.hidden) return;
      if (lastDraw && now - lastDraw < 1000 / 30) {
        frame = requestAnimationFrame(animate);
        return;
      }
      try {
        lastDraw = now;
        const dt = lastFrame ? Math.min((now - lastFrame) / 1000, 0.05) : 1 / 60;
        lastFrame = now;
        if (!reduceMotion) time += dt * 0.5;
        draw(now);
        if (!reduceMotion || now - clickAt < 1200 || now - pointerAt < 90) {
          frame = requestAnimationFrame(animate);
        } else {
          lastFrame = 0;
        }
      } catch {
        failRenderer();
      }
    };

    const wake = () => {
      if (active && !frame && !disposed && !contextLost && visible && !document.hidden && !reduceMotion) {
        frame = requestAnimationFrame(animate);
      }
    };

    const renderStatic = () => {
      if (!active || disposed || !visible || document.hidden || !reduceMotion || !gl || contextLost) {
        if (!reduceMotion) wake();
        return;
      }
      if (gl && !contextLost) {
        try {
          draw(performance.now());
        } catch {
          failRenderer();
        }
      }
    };

    const interactionRoot = container.parentElement ?? container;
    const locate = (event: PointerEvent): Point | null => {
      const rect = container.getBoundingClientRect();
      if (
        rect.width <= 0 ||
        rect.height <= 0 ||
        event.clientX < rect.left ||
        event.clientX > rect.right ||
        event.clientY < rect.top ||
        event.clientY > rect.bottom
      )
        return null;
      return {
        x: ((event.clientX - rect.left) / rect.width - 0.5) * (rect.width / rect.height),
        y: 0.5 - (event.clientY - rect.top) / rect.height,
      };
    };
    const onPointerMove = (event: PointerEvent) => {
      if (reduceMotion) return;
      const point = locate(event);
      if (!point) return;
      pointer = point;
      pointerAt = performance.now();
      wake();
    };
    const onPointerDown = (event: PointerEvent) => {
      if (reduceMotion) return;
      const point = locate(event);
      if (!point) return;
      pointer = point;
      pointerAt = -Infinity;
      clickAt = performance.now();
      wake();
    };
    const onVisibility = () => {
      lastFrame = 0;
      if (document.hidden) stopFrame();
      else renderStatic();
    };
    const onMotionChange = (event: MediaQueryListEvent) => {
      reduceMotion = event.matches;
      pointerAt = -Infinity;
      clickAt = -Infinity;
      stopFrame();
      renderStatic();
    };
    const resize = () => {
      if (reduceMotion) renderStatic();
      else wake();
    };
    const onIntersect: IntersectionObserverCallback = (entries) => {
      visible = entries.some((entry) => entry.isIntersecting);
      if (!visible) stopFrame();
      else renderStatic();
    };

    let resizeObserver: ResizeObserver | null = null;
    let intersectionObserver: IntersectionObserver | null = null;
    let listenersAttached = false;
    const detachListeners = () => {
      resizeObserver?.disconnect();
      intersectionObserver?.disconnect();
      if (listenersAttached) {
        interactionRoot.removeEventListener("pointermove", onPointerMove);
        interactionRoot.removeEventListener("pointerdown", onPointerDown);
        document.removeEventListener("visibilitychange", onVisibility);
        motionQuery?.removeEventListener?.("change", onMotionChange);
        window.removeEventListener("resize", resize);
        listenersAttached = false;
      }
    };
    failRenderer = () => {
      active = false;
      fallback();
      detachListeners();
      cleanupResources();
    };
    try {
      gl = canvas.getContext("webgl2", { alpha: true, antialias: false, depth: false, stencil: false });
      if (!gl) throw new Error("WebGL2 is unavailable");
      programInfo = createProgram(gl);
      atlas = createAtlas(gl);
      vao = gl.createVertexArray();
      if (!vao) throw new Error("Unable to allocate ASCII vertex array");
      gl.bindVertexArray(vao);
      buffer = gl.createBuffer();
      if (!buffer) throw new Error("Unable to allocate ASCII vertex buffer");
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
      const position = gl.getAttribLocation(programInfo.program, "aPosition");
      if (position < 0) throw new Error("Missing ASCII vertex attribute");
      gl.enableVertexAttribArray(position);
      gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
      resizeObserver = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(resize);
      resizeObserver?.observe(container);
      intersectionObserver = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver(onIntersect);
      intersectionObserver?.observe(container);
      listenersAttached = true;
      interactionRoot.addEventListener("pointermove", onPointerMove, { passive: true });
      interactionRoot.addEventListener("pointerdown", onPointerDown, { passive: true });
      document.addEventListener("visibilitychange", onVisibility);
      motionQuery?.addEventListener?.("change", onMotionChange);
      window.addEventListener("resize", resize, { passive: true });
      active = true;
      resize();
      renderStatic();
      if (document.fonts) {
        const renderer = gl;
        void document.fonts
          .load('300 78px "Geist Mono"', CHARACTERS)
          .then(() => {
            if (!active || disposed || contextLost || !renderer || gl !== renderer) return;
            fontReady = true;
            if (!visible || document.hidden) return;
            if (reduceMotion) renderStatic();
            else wake();
          })
          .catch(() => {
            // The initial atlas remains usable with the system monospace fallback.
          });
      }
    } catch {
      failRenderer();
    }

    return () => {
      disposed = true;
      active = false;
      stopFrame();
      detachListeners();
      canvas.removeEventListener("webglcontextlost", onContextLost);
      cleanupResources();
      delete container.dataset.asciiReady;
    };
  }, []);

  return (
    <div ref={containerRef} className={["gradient-ascii", className].filter(Boolean).join(" ")} aria-hidden="true">
      <canvas ref={canvasRef} className="gradient-ascii__canvas" aria-hidden="true" />
    </div>
  );
}
