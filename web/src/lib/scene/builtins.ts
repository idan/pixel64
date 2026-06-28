// Built-in functions + value helpers for the shader interpreter.
//
// Values are either a scalar `number` or a `number[]` of length 2/3/4 (vec2/3/4).
// Booleans are represented as 0/1. This dynamic representation is the spike's
// shortcut; the device compiler instead lowers vectors to scalar ops (see
// docs/scenes/shader-vm.md). Math semantics here mirror GLSL: component-wise
// ops with scalar↔vector broadcasting.

export type Value = number | number[];

export class RuntimeError extends Error {
	constructor(message: string) {
		super(message);
		this.name = 'RuntimeError';
	}
}

export const isVec = (v: Value): v is number[] => Array.isArray(v);

const TAU = Math.PI * 2;

// Fast sine, mirroring renderer/src/vm.rs::fast_sin so this reference matches the Rust→WASM renderer.
// The renderer replaced libm's sinf (which range-reduces in f64 — catastrophic on the device's
// single-precision FPU) with this degree-9 odd-Taylor approximation over [-π/2, π/2]. Keeping the
// same polynomial here keeps the editor's WASM-vs-TS parity exact on sin/cos scenes. See
// docs/scenes/shader-vm.md. (Evaluated in f64 here vs f32 on device/WASM — that gap is far under the
// 1/255 output quantization, as for every other op in this reference.)
const HALF_PI = Math.PI / 2;
// Round half away from zero, matching Rust's libm::roundf (JS Math.round rounds half toward +∞).
const roundTiesAway = (x: number) => Math.sign(x) * Math.round(Math.abs(x));
function fastSin(x: number): number {
	let r = x - TAU * roundTiesAway(x / TAU);
	if (r > HALF_PI) r = Math.PI - r;
	else if (r < -HALF_PI) r = -Math.PI - r;
	const r2 = r * r;
	return r * (1 + r2 * (-1 / 6 + r2 * (1 / 120 + r2 * (-1 / 5040 + r2 * (1 / 362880)))));
}
const fastCos = (x: number): number => fastSin(x + HALF_PI);

/** Coerce to a scalar, erroring on vectors. */
export function asScalar(v: Value, what = 'value'): number {
	if (isVec(v)) throw new RuntimeError(`Expected a scalar ${what}, got a vec${v.length}`);
	return v;
}

/** Apply a scalar→scalar fn component-wise across a scalar or vector. */
function map1(fn: (x: number) => number, v: Value): Value {
	return isVec(v) ? v.map(fn) : fn(v);
}

/** Apply a scalar fn pairwise with scalar↔vector broadcasting. */
function map2(fn: (a: number, b: number) => number, a: Value, b: Value): Value {
	if (isVec(a) && isVec(b)) {
		if (a.length !== b.length)
			throw new RuntimeError(`Mismatched vec sizes: vec${a.length} vs vec${b.length}`);
		return a.map((x, i) => fn(x, b[i]));
	}
	if (isVec(a)) return a.map((x) => fn(x, b as number));
	if (isVec(b)) return b.map((y) => fn(a as number, y));
	return fn(a, b);
}

/** Apply a scalar fn across three operands with broadcasting (clamp/mix/etc.). */
function map3(
	fn: (a: number, b: number, c: number) => number,
	a: Value,
	b: Value,
	c: Value
): Value {
	const len = [a, b, c].find(isVec) as number[] | undefined;
	if (!len) return fn(a as number, b as number, c as number);
	const n = len.length;
	const at = (v: Value, i: number) => (isVec(v) ? v[i] : v);
	const out: number[] = [];
	for (let i = 0; i < n; i++) out.push(fn(at(a, i), at(b, i), at(c, i)));
	return out;
}

// ---- exported arithmetic used by the interpreter for operators ----

export const add = (a: Value, b: Value) => map2((x, y) => x + y, a, b);
export const sub = (a: Value, b: Value) => map2((x, y) => x - y, a, b);
export const mul = (a: Value, b: Value) => map2((x, y) => x * y, a, b);
export const div = (a: Value, b: Value) => map2((x, y) => x / y, a, b);
export const fmod = (a: Value, b: Value) => map2((x, y) => x - y * Math.floor(x / y), a, b);
export const neg = (a: Value) => map1((x) => -x, a);

// ---- geometry ----

const lengthOf = (v: Value): number => {
	if (!isVec(v)) return Math.abs(v);
	let s = 0;
	for (const x of v) s += x * x;
	return Math.sqrt(s);
};
const dot = (a: Value, b: Value): number => {
	if (!isVec(a) || !isVec(b)) return asScalar(a) * asScalar(b);
	let s = 0;
	for (let i = 0; i < a.length; i++) s += a[i] * b[i];
	return s;
};

// ---- color: IQ cosine palette + hsv ----

// iq palette: a + b*cos(2π(c*t + d)). Defaults give a pleasant rainbow.
const palette = (t: number, a?: Value, b?: Value, c?: Value, d?: Value): number[] => {
	const A = (a as number[]) ?? [0.5, 0.5, 0.5];
	const B = (b as number[]) ?? [0.5, 0.5, 0.5];
	const C = (c as number[]) ?? [1.0, 1.0, 1.0];
	const D = (d as number[]) ?? [0.0, 0.33, 0.67];
	return [0, 1, 2].map((i) => A[i] + B[i] * fastCos(TAU * (C[i] * t + D[i])));
};

const hsv = (h: number, s: number, v: number): number[] => {
	h = ((h % 1) + 1) % 1;
	const i = Math.floor(h * 6);
	const f = h * 6 - i;
	const p = v * (1 - s);
	const q = v * (1 - f * s);
	const tt = v * (1 - (1 - f) * s);
	switch (i % 6) {
		case 0:
			return [v, tt, p];
		case 1:
			return [q, v, p];
		case 2:
			return [p, v, tt];
		case 3:
			return [p, q, v];
		case 4:
			return [tt, p, v];
		default:
			return [v, p, q];
	}
};

// ---- value noise (integer-hash, smoothstep-interpolated) ----
//
// Integer bit-mix hash (lowbias32) over integer lattice coords, mirroring
// renderer/src/vm.rs. `Math.imul` reproduces Rust's wrapping u32 multiply, so
// noise is bit-portable between the JS preview and the device (the sine-based
// hash this replaced diverged badly under f32 — see docs/scenes/shader-vm.md).

const fract = (x: number) => x - Math.floor(x);
const _fb = new Float32Array(1);
const _ib = new Uint32Array(_fb.buffer);
function imix(h: number): number {
	h = h >>> 0;
	h ^= h >>> 16;
	h = Math.imul(h, 0x7feb352d);
	h ^= h >>> 15;
	h = Math.imul(h, 0x846ca68b);
	h ^= h >>> 16;
	return h >>> 0;
}
const toUnit = (u: number) => (u >>> 8) / 16777216; // top 24 bits → [0,1)
const hash1 = (x: number) => {
	_fb[0] = x;
	return toUnit(imix(_ib[0]));
};
const hash2 = (x: number, y: number) =>
	toUnit(imix(Math.imul(Math.floor(x) | 0, 0x8da6b343) ^ Math.imul(Math.floor(y) | 0, 0xd8163841)));
const hash3 = (x: number, y: number, z: number) =>
	toUnit(
		imix(
			Math.imul(Math.floor(x) | 0, 0x8da6b343) ^
				Math.imul(Math.floor(y) | 0, 0xd8163841) ^
				Math.imul(Math.floor(z) | 0, 0xcb1ab31f)
		)
	);

const smooth = (f: number) => f * f * (3 - 2 * f);
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

const noise2 = (p: number[]): number => {
	const ix = Math.floor(p[0]);
	const iy = Math.floor(p[1]);
	const fx = smooth(p[0] - ix);
	const fy = smooth(p[1] - iy);
	const a = hash2(ix, iy);
	const b = hash2(ix + 1, iy);
	const c = hash2(ix, iy + 1);
	const d = hash2(ix + 1, iy + 1);
	return lerp(lerp(a, b, fx), lerp(c, d, fx), fy);
};

const noise3 = (p: number[]): number => {
	const ix = Math.floor(p[0]);
	const iy = Math.floor(p[1]);
	const iz = Math.floor(p[2]);
	const fx = smooth(p[0] - ix);
	const fy = smooth(p[1] - iy);
	const fz = smooth(p[2] - iz);
	const c = (dx: number, dy: number, dz: number) => hash3(ix + dx, iy + dy, iz + dz);
	const z0 = lerp(lerp(c(0, 0, 0), c(1, 0, 0), fx), lerp(c(0, 1, 0), c(1, 1, 0), fx), fy);
	const z1 = lerp(lerp(c(0, 0, 1), c(1, 0, 1), fx), lerp(c(0, 1, 1), c(1, 1, 1), fx), fy);
	return lerp(z0, z1, fz);
};

// ---- builtin registry ----

type Fn = (args: Value[]) => Value;

const s1 = (fn: (x: number) => number): Fn => (a) => map1(fn, a[0]);

export const BUILTINS: Record<string, Fn> = {
	// common math (component-wise)
	abs: s1(Math.abs),
	floor: s1(Math.floor),
	ceil: s1(Math.ceil),
	fract: s1(fract),
	sign: s1(Math.sign),
	sqrt: s1(Math.sqrt),
	exp: s1(Math.exp),
	log: s1(Math.log),
	radians: s1((d) => (d * Math.PI) / 180),
	degrees: s1((r) => (r * 180) / Math.PI),

	min: (a) => map2(Math.min, a[0], a[1]),
	max: (a) => map2(Math.max, a[0], a[1]),
	mod: (a) => fmod(a[0], a[1]),
	pow: (a) => map2(Math.pow, a[0], a[1]),
	step: (a) => map2((edge, x) => (x < edge ? 0 : 1), a[0], a[1]),
	clamp: (a) => map3((x, lo, hi) => Math.min(Math.max(x, lo), hi), a[0], a[1], a[2]),
	mix: (a) => map3((x, y, t) => x + (y - x) * t, a[0], a[1], a[2]),
	smoothstep: (a) =>
		map3(
			(e0, e1, x) => {
				const t = Math.min(Math.max((x - e0) / (e1 - e0), 0), 1);
				return t * t * (3 - 2 * t);
			},
			a[0],
			a[1],
			a[2]
		),

	// trig — sin/cos use the same fast polynomial as the renderer (see fastSin above) so preview and
	// device agree; tan/atan still use Math (renderer keeps libm for those).
	sin: s1(fastSin),
	cos: s1(fastCos),
	tan: s1(Math.tan),
	atan: (a) => (a.length === 2 ? Math.atan2(asScalar(a[0]), asScalar(a[1])) : map1(Math.atan, a[0])),

	// geometry
	length: (a) => lengthOf(a[0]),
	distance: (a) => lengthOf(sub(a[0], a[1])),
	dot: (a) => dot(a[0], a[1]),
	normalize: (a) => {
		const l = lengthOf(a[0]);
		return l === 0 ? a[0] : div(a[0], l);
	},
	cross: (a) => {
		const u = a[0] as number[];
		const v = a[1] as number[];
		return [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
	},

	// color
	hsv: (a) => hsv(asScalar(a[0]), asScalar(a[1]), asScalar(a[2])),
	palette: (a) => palette(asScalar(a[0]), a[1], a[2], a[3], a[4]),

	// noise
	hash: (a) => hash1(asScalar(a[0])),
	noise: (a) => {
		const p = a[0];
		if (!isVec(p)) throw new RuntimeError('noise() expects a vec2 or vec3');
		return p.length === 3 ? noise3(p) : noise2(p);
	},

	// casts / constructors handled here for the scalar cast; vec* in interpreter
	float: (a) => asScalar(a[0])
};

// fbm is a "prelude" function: fractal sum of noise over a constant octave count.
export function fbm(p: number[], octaves: number): number {
	let sum = 0;
	let amp = 0.5;
	let freq = 1;
	const n = Math.max(1, Math.floor(octaves));
	let pp = p.slice();
	for (let i = 0; i < n; i++) {
		const v = pp.length === 3 ? noise3(pp) : noise2(pp);
		sum += amp * v;
		amp *= 0.5;
		freq *= 2;
		pp = pp.map((c) => c * 2);
	}
	return sum;
}

/** Vector constructor with GLSL splat/concat semantics. */
export function makeVec(n: 2 | 3 | 4, args: Value[]): number[] {
	// splat: vec3(x) -> [x,x,x]
	if (args.length === 1 && !isVec(args[0])) {
		return new Array(n).fill(args[0] as number);
	}
	// flatten scalars + vectors left-to-right (vec4(rgb, a), vec3(xy, z), ...)
	const out: number[] = [];
	for (const a of args) {
		if (isVec(a)) out.push(...a);
		else out.push(a);
	}
	if (out.length !== n)
		throw new RuntimeError(`vec${n}() needs ${n} components, got ${out.length}`);
	return out;
}
