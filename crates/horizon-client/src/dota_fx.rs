//! Dota's world particles: the map's dota_world_particle_system entities
//! (fireflies, river flow, lamp and candle flames, waterfalls, lava) from
//! particles.json (tools/extract_dota_particles.py), simulated on the CPU with
//! Source 2's operators and drawn as sprites inside the map pass.
//!
//! Particles live in Source units (inches, Z up) relative to their system's
//! control point 0; drawing turns them into map space (metres, Y up) with the
//! entity's rotation. Sprite size follows the renderer: the radius in inches,
//! clamped to m_flMinSize..m_flMaxSize of the screen and faded out between
//! m_flStartFadeSize and m_flEndFadeSize, the way particles.dll draws them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::Value;

const INCH: f32 = 0.0254;
/// Simulation step: particles.dll runs its operators at 30 Hz or faster.
const STEP: f32 = 1.0 / 60.0;
/// Systems farther than this from the camera (metres) are not simulated.
const DEFAULT_DRAW_DISTANCE: f32 = 90.0;
/// At most this many particles per system instance, whatever m_nMaxParticles says.
const PARTICLE_CAP: usize = 256;

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn len(a: V3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn lerp3(a: V3, b: V3, t: V3) -> V3 {
    [lerp(a[0], b[0], t[0]), lerp(a[1], b[1], t[1]), lerp(a[2], b[2], t[2])]
}

/// A small xorshift generator: each system instance has its own.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        lerp(a, b, self.next())
    }
    fn v3(&mut self, a: V3, b: V3) -> V3 {
        [self.range(a[0], b[0]), self.range(a[1], b[1]), self.range(a[2], b[2])]
    }
}

// ---------------------------------------------------------------- parsing --

fn f(v: &Value, key: &str, default: f32) -> f32 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_f64().map_or(default, |x| x as f32),
        Some(obj @ Value::Object(_)) => FloatIn::parse(obj).literal().unwrap_or(default),
        _ => default,
    }
}
fn i(v: &Value, key: &str, default: i64) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(default)
}
fn b(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn v3(v: &Value, key: &str, default: V3) -> V3 {
    v.get(key).map_or(default, |x| vec_value(x, default))
}

/// A vector parameter: a plain array or a CPerParticleVecInput object.
fn vec_value(x: &Value, default: V3) -> V3 {
    let array = |a: &Value, d: V3| -> Option<V3> {
        let a = a.as_array()?;
        let c = |k: usize| a.get(k).and_then(Value::as_f64).map_or(d[k], |x| x as f32);
        (a.len() >= 3).then(|| [c(0), c(1), c(2)])
    };
    if let Some(a) = array(x, default) {
        return a;
    }
    match s(x, "m_nType") {
        "PVEC_TYPE_LITERAL_COLOR" => array(x.get("m_LiteralColor").unwrap_or(&Value::Null), default).unwrap_or(default),
        "PVEC_TYPE_FLOAT_COMPONENTS" => [
            f(x, "m_FloatComponentX", default[0]),
            f(x, "m_FloatComponentY", default[1]),
            f(x, "m_FloatComponentZ", default[2]),
        ],
        "PVEC_TYPE_RANDOM_UNIFORM_OFFSET" | "PVEC_TYPE_RANDOM_UNIFORM" => {
            let base = array(x.get("m_vLiteralValue").unwrap_or(&Value::Null), [0.0; 3]).unwrap_or([0.0; 3]);
            let lo = array(x.get("m_vRandomMin").unwrap_or(&Value::Null), [0.0; 3]).unwrap_or([0.0; 3]);
            let hi = array(x.get("m_vRandomMax").unwrap_or(&Value::Null), [0.0; 3]).unwrap_or([0.0; 3]);
            add(base, mul(add(lo, hi), 0.5))
        }
        _ => array(x.get("m_vLiteralValue").unwrap_or(&Value::Null), default).unwrap_or(default),
    }
}
fn color(v: &Value, key: &str, default: [f32; 4]) -> [f32; 4] {
    match v.get(key).and_then(Value::as_array) {
        Some(a) if a.len() >= 3 => {
            let c = |k: usize, d: f32| a.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32);
            [c(0, default[0]), c(1, default[1]), c(2, default[2]), c(3, default[3])]
        }
        _ => default,
    }
}

/// A CPerParticleFloatInput: the kinds the map's systems use.
#[derive(Clone, Debug)]
enum FloatIn {
    Literal(f32),
    Random(f32, f32),
    /// PF_TYPE_PARTICLE_AGE_NORMALIZED and friends, remapped linearly or along a curve.
    AgeNormalized(Remap),
    Age(Remap),
}

#[derive(Clone, Debug)]
struct Remap {
    input: (f32, f32),
    output: (f32, f32),
    curve: Vec<(f32, f32)>,
}

impl Remap {
    fn eval(&self, x: f32) -> f32 {
        if self.curve.len() >= 2 {
            let c = &self.curve;
            if x <= c[0].0 {
                return c[0].1;
            }
            for w in c.windows(2) {
                if x <= w[1].0 {
                    let t = (x - w[0].0) / (w[1].0 - w[0].0).max(1.0e-6);
                    return lerp(w[0].1, w[1].1, t);
                }
            }
            return c[c.len() - 1].1;
        }
        let (i0, i1) = self.input;
        let t = ((x - i0) / (i1 - i0).max(1.0e-6)).clamp(0.0, 1.0);
        lerp(self.output.0, self.output.1, t)
    }
}

impl FloatIn {
    fn parse(v: &Value) -> FloatIn {
        if let Some(n) = v.as_f64() {
            return FloatIn::Literal(n as f32);
        }
        let remap = || Remap {
            input: (f(v, "m_flInput0", 0.0), f(v, "m_flInput1", 1.0)),
            output: (f(v, "m_flOutput0", 0.0), f(v, "m_flOutput1", 1.0)),
            curve: v
                .get("m_Curve")
                .and_then(|c| c.get("m_spline"))
                .and_then(Value::as_array)
                .map(|pts| {
                    pts.iter()
                        .filter_map(|p| Some((p.get("x")?.as_f64()? as f32, p.get("y")?.as_f64()? as f32)))
                        .collect()
                })
                .unwrap_or_default(),
        };
        match s(v, "m_nType") {
            "PF_TYPE_RANDOM_UNIFORM" | "PF_TYPE_RANDOM_BIASED" => {
                FloatIn::Random(f(v, "m_flRandomMin", 0.0), f(v, "m_flRandomMax", 1.0))
            }
            "PF_TYPE_PARTICLE_AGE_NORMALIZED" => FloatIn::AgeNormalized(remap()),
            "PF_TYPE_PARTICLE_AGE" => FloatIn::Age(remap()),
            _ => FloatIn::Literal(f(v, "m_flLiteralValue", 0.0)),
        }
    }
    fn literal(&self) -> Option<f32> {
        match self {
            FloatIn::Literal(x) => Some(*x),
            FloatIn::Random(a, b) => Some((a + b) * 0.5),
            _ => None,
        }
    }
    fn eval(&self, p: &P, rng: &mut Rng) -> f32 {
        match self {
            FloatIn::Literal(x) => *x,
            FloatIn::Random(a, b) => rng.range(*a, *b),
            FloatIn::AgeNormalized(r) => r.eval(p.age / p.life.max(1.0e-4)),
            FloatIn::Age(r) => r.eval(p.age),
        }
    }
}

fn float_in(v: &Value, key: &str, default: f32) -> FloatIn {
    v.get(key).map_or(FloatIn::Literal(default), FloatIn::parse)
}

// ------------------------------------------------------------ definitions --

/// Particle attribute numbers (m_nOutputField, m_nField) that matter here.
const F_LIFETIME: i64 = 1;
const F_RADIUS: i64 = 3;
const F_ROLL: i64 = 4;
const F_ROLL_SPEED: i64 = 5;
const F_ALPHA: i64 = 7;
const F_SEQUENCE: i64 = 9;
const F_YAW: i64 = 12;
const F_ALPHA2: i64 = 16;

#[derive(Clone, Debug)]
enum Emitter {
    Continuous { rate: FloatIn, start: f32, duration: f32 },
    Instant { count: FloatIn, start: f32 },
}

#[derive(Clone, Debug)]
enum Init {
    Float { field: i64, value: FloatIn, scale: bool },
    Sphere { rmin: f32, rmax: f32, bias: V3, smin: f32, smax: f32, lmin: V3, lmax: V3 },
    Offset { min: V3, max: V3, local: bool },
    Color { min: [f32; 4], max: [f32; 4] },
    Sequence { min: f32, max: f32 },
    VelocityNoise { min: V3, max: V3, local: bool },
    VelocityRandom { smin: f32, smax: f32, lmin: V3, lmax: V3 },
    Scalar { field: i64, min: f32, max: f32 },
    Path { end: usize, bulge_cp: usize, bulge: f32, spread: f32 },
    FromParent,
}

#[derive(Clone, Debug)]
enum Op {
    Decay,
    Movement { gravity: V3, drag: f32 },
    Radius { t0: f32, t1: f32, s0: f32, s1: f32, bias: f32 },
    FadeInSimple(f32),
    FadeOutSimple(f32),
    FadeIn { min: f32, max: f32, proportional: bool },
    FadeOut { min: f32, max: f32, proportional: bool },
    FadeAndKill { a0: f32, a1: f32, in0: f32, in1: f32, out0: f32, out1: f32 },
    ColorTo { target: V3, t0: f32, t1: f32 },
    ColorToRandom { min: V3, max: V3, t0: f32, t1: f32 },
    SpinUpdate,
    Spin { rate: f32, min: f32 },
    SetFloat { field: i64, value: FloatIn, scale: bool },
    Oscillate { field: i64, rmin: f32, rmax: f32, fmin: f32, fmax: f32, t0: f32, t1: f32, proportional: bool },
    OscillateVector { rmin: V3, rmax: V3, fmin: V3, fmax: V3, t0: f32, t1: f32 },
    VectorNoise { min: V3, max: V3, scale: f32, additive: bool },
    Noise { field: i64, min: f32, max: f32, scale: f32 },
    Ramp { field: i64, rmin: f32, rmax: f32, t0: f32, t1: f32 },
    MaxVelocity(f32),
    OrientTo2d(f32),
    Clamp { field: i64, min: f32, max: f32 },
    RandomForce { min: V3, max: V3 },
    Wind,
    Attract { amount: f32, power: f32, offset: V3 },
    Twist { amount: f32, axis: V3, local: bool },
    Constrain { min: f32, max: f32, offset: V3 },
    Plane { point: V3, normal: V3 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Blend {
    Alpha,
    Add,
    Mod2x,
}

#[derive(Clone, Debug)]
struct Sprite {
    texture: Option<usize>,
    blend: Blend,
    overbright: f32,
    add_self: f32,
    rate: f32,
    fps: bool,
    fit: bool,
    flat: bool,
    min_size: f32,
    max_size: f32,
    fade_start: f32,
    fade_end: f32,
    alpha_scale: f32,
    global_light: bool,
    no_depth: bool,
}

#[derive(Clone, Debug)]
struct LightDef {
    radius_scale: f32,
    alpha_scale: f32,
    color: V3,
}

#[derive(Clone, Debug)]
struct ModelDef {
    /// The model's file stem (wildlife_butterfly001).
    stem: String,
    clip: &'static str,
    /// Clip seconds per second of particle age.
    rate: f32,
}

#[derive(Clone, Debug)]
enum Render {
    None,
    Sprite(Sprite),
    Light(LightDef),
    Model(ModelDef),
}

#[derive(Clone, Debug)]
struct System {
    name: String,
    max: usize,
    radius: f32,
    lifetime: f32,
    color: [f32; 4],
    emitters: Vec<Emitter>,
    inits: Vec<Init>,
    ops: Vec<Op>,
    renders: Vec<Render>,
    children: Vec<(usize, f32)>,
    draw_distance: f32,
    /// Control points set before emission (C_OP_SetSingleControlPointPosition), local to CP 0.
    cps: Vec<(usize, V3)>,
    /// C_OP_RestartAfterDuration: restart the children every min..max seconds.
    restart: Option<(f32, f32)>,
}

struct Sheet {
    sequences: Vec<SheetSequence>,
}

struct SheetSequence {
    /// (end of the frame in sequence time, uv rect)
    frames: Vec<(f32, [f32; 4])>,
    total: f32,
    clamp: bool,
}

struct Library {
    systems: Vec<System>,
    textures: Vec<(PathBuf, Sheet)>,
    instances: Vec<Placement>,
}

struct Placement {
    system: usize,
    pos: V3,
    rot: [[f32; 3]; 3],
    day: bool,
    night: bool,
}

fn parse_emitter(e: &Value) -> Option<Emitter> {
    match s(e, "_class") {
        "C_OP_ContinuousEmitter" => Some(Emitter::Continuous {
            rate: float_in(e, "m_flEmitRate", 100.0),
            start: f(e, "m_flStartTime", 0.0),
            duration: f(e, "m_flEmissionDuration", 0.0),
        }),
        "C_OP_NoiseEmitter" => Some(Emitter::Continuous {
            rate: FloatIn::Literal((f(e, "m_flOutputMin", 0.0) + f(e, "m_flOutputMax", 100.0)) * 0.5),
            start: f(e, "m_flStartTime", 0.0),
            duration: f(e, "m_flEmissionDuration", 0.0),
        }),
        "C_OP_InstantaneousEmitter" => Some(Emitter::Instant {
            count: float_in(e, "m_nParticlesToEmit", 100.0),
            start: f(e, "m_flStartTime", 0.0),
        }),
        _ => None,
    }
}

fn parse_init(v: &Value) -> Option<Init> {
    Some(match s(v, "_class") {
        "C_INIT_InitFloat" => Init::Float {
            field: i(v, "m_nOutputField", F_RADIUS),
            value: float_in(v, "m_InputValue", 0.0),
            scale: s(v, "m_nSetMethod") == "PARTICLE_SET_SCALE_INITIAL_VALUE",
        },
        "C_INIT_CreateWithinSphere" | "C_INIT_CreateWithinSphereTransform" => Init::Sphere {
            rmin: f(v, "m_fRadiusMin", 0.0),
            rmax: f(v, "m_fRadiusMax", 0.0),
            bias: v3(v, "m_vecDistanceBias", [1.0, 1.0, 1.0]),
            smin: f(v, "m_fSpeedMin", 0.0),
            smax: f(v, "m_fSpeedMax", 0.0),
            lmin: v3(v, "m_LocalCoordinateSystemSpeedMin", [0.0; 3]),
            lmax: v3(v, "m_LocalCoordinateSystemSpeedMax", [0.0; 3]),
        },
        "C_INIT_PositionOffset" => Init::Offset {
            min: v3(v, "m_OffsetMin", [0.0; 3]),
            max: v3(v, "m_OffsetMax", [0.0; 3]),
            local: b(v, "m_bLocalCoords", false),
        },
        "C_INIT_RandomColor" => Init::Color {
            min: color(v, "m_ColorMin", [255.0; 4]),
            max: color(v, "m_ColorMax", [255.0; 4]),
        },
        "C_INIT_RandomSequence" => Init::Sequence {
            min: i(v, "m_nSequenceMin", 0) as f32,
            max: i(v, "m_nSequenceMax", 0) as f32,
        },
        "C_INIT_InitialVelocityNoise" => Init::VelocityNoise {
            min: v3(v, "m_vecOutputMin", [0.0; 3]),
            max: v3(v, "m_vecOutputMax", [0.0; 3]),
            local: b(v, "m_bLocalSpace", false),
        },
        "C_INIT_VelocityRandom" => Init::VelocityRandom {
            smin: f(v, "m_fSpeedMin", 0.0),
            smax: f(v, "m_fSpeedMax", 0.0),
            lmin: v3(v, "m_LocalCoordinateSystemSpeedMin", [0.0; 3]),
            lmax: v3(v, "m_LocalCoordinateSystemSpeedMax", [0.0; 3]),
        },
        "C_INIT_CreationNoise" => Init::Scalar {
            field: i(v, "m_nFieldOutput", F_RADIUS),
            min: f(v, "m_flOutputMin", 0.0),
            max: f(v, "m_flOutputMax", 1.0),
        },
        "C_INIT_CreateAlongPath" => {
            let path = v.get("m_PathParams").unwrap_or(&Value::Null);
            Init::Path {
                end: i(path, "m_nEndControlPointNumber", 0).clamp(0, 7) as usize,
                bulge_cp: i(path, "m_nBulgeControl", 0).clamp(0, 7) as usize,
                bulge: f(path, "m_flBulge", 0.0),
                spread: f(v, "m_fMaxDistance", 0.0),
            }
        }
        "C_INIT_CreateFromParentParticles" => Init::FromParent,
        _ => return None,
    })
}

fn parse_op(v: &Value) -> Option<Op> {
    Some(match s(v, "_class") {
        "C_OP_Decay" | "C_OP_EndCapTimedDecay" => Op::Decay,
        "C_OP_BasicMovement" => Op::Movement {
            gravity: v3(v, "m_Gravity", [0.0; 3]),
            drag: f(v, "m_fDrag", 0.0),
        },
        "C_OP_InterpolateRadius" => Op::Radius {
            t0: f(v, "m_flStartTime", 0.0),
            t1: f(v, "m_flEndTime", 1.0),
            s0: f(v, "m_flStartScale", 1.0),
            s1: f(v, "m_flEndScale", 1.0),
            bias: f(v, "m_flBias", 0.5),
        },
        "C_OP_FadeInSimple" => Op::FadeInSimple(f(v, "m_flFadeInTime", 0.25)),
        "C_OP_FadeOutSimple" => Op::FadeOutSimple(f(v, "m_flFadeOutTime", 0.25)),
        "C_OP_FadeIn" => Op::FadeIn {
            min: f(v, "m_flFadeInTimeMin", 0.25),
            max: f(v, "m_flFadeInTimeMax", 0.25),
            proportional: b(v, "m_bProportional", true),
        },
        "C_OP_FadeOut" => Op::FadeOut {
            min: f(v, "m_flFadeOutTimeMin", 0.25),
            max: f(v, "m_flFadeOutTimeMax", 0.25),
            proportional: b(v, "m_bProportional", true),
        },
        "C_OP_FadeAndKill" => Op::FadeAndKill {
            a0: f(v, "m_flStartAlpha", 1.0),
            a1: f(v, "m_flEndAlpha", 0.0),
            in0: f(v, "m_flStartFadeInTime", 0.0),
            in1: f(v, "m_flEndFadeInTime", 0.5),
            out0: f(v, "m_flStartFadeOutTime", 0.5),
            out1: f(v, "m_flEndFadeOutTime", 1.0),
        },
        "C_OP_ColorInterpolate" => {
            let c = color(v, "m_ColorFade", [255.0; 4]);
            Op::ColorTo {
                target: [c[0], c[1], c[2]],
                t0: f(v, "m_flFadeStartTime", 0.0),
                t1: f(v, "m_flFadeEndTime", 1.0),
            }
        }
        "C_OP_ColorInterpolateRandom" => {
            let a = color(v, "m_ColorFadeMin", [255.0; 4]);
            let c = color(v, "m_ColorFadeMax", [255.0; 4]);
            Op::ColorToRandom {
                min: [a[0], a[1], a[2]],
                max: [c[0], c[1], c[2]],
                t0: f(v, "m_flFadeStartTime", 0.0),
                t1: f(v, "m_flFadeEndTime", 1.0),
            }
        }
        "C_OP_SpinUpdate" => Op::SpinUpdate,
        "C_OP_Spin" => Op::Spin {
            rate: f(v, "m_nSpinRateDegrees", 0.0),
            min: f(v, "m_nSpinRateMinDegrees", 0.0),
        },
        "C_OP_SetFloat" => Op::SetFloat {
            field: i(v, "m_nOutputField", F_RADIUS),
            value: float_in(v, "m_InputValue", 0.0),
            scale: s(v, "m_nSetMethod").starts_with("PARTICLE_SET_SCALE"),
        },
        "C_OP_OscillateScalar" => Op::Oscillate {
            field: i(v, "m_nField", F_ALPHA),
            rmin: f(v, "m_RateMin", 0.0),
            rmax: f(v, "m_RateMax", 0.0),
            fmin: f(v, "m_FrequencyMin", 1.0),
            fmax: f(v, "m_FrequencyMax", 1.0),
            t0: f(v, "m_flStartTime_min", 0.0),
            t1: f(v, "m_flEndTime_max", 1.0),
            proportional: b(v, "m_bProportional", true),
        },
        "C_OP_OscillateVector" => Op::OscillateVector {
            rmin: v3(v, "m_RateMin", [0.0; 3]),
            rmax: v3(v, "m_RateMax", [0.0; 3]),
            fmin: v3(v, "m_FrequencyMin", [1.0; 3]),
            fmax: v3(v, "m_FrequencyMax", [1.0; 3]),
            t0: f(v, "m_flStartTime_min", 0.0),
            t1: f(v, "m_flEndTime_max", 1.0),
        },
        "C_OP_VectorNoise" if i(v, "m_nFieldOutput", 0) == 0 => Op::VectorNoise {
            min: v3(v, "m_vecOutputMin", [0.0; 3]),
            max: v3(v, "m_vecOutputMax", [1.0; 3]),
            scale: f(v, "m_fl4NoiseScale", 0.01),
            additive: b(v, "m_bAdditive", false),
        },
        "C_OP_Noise" => Op::Noise {
            field: i(v, "m_nFieldOutput", F_RADIUS),
            min: f(v, "m_flOutputMin", 0.0),
            max: f(v, "m_flOutputMax", 1.0),
            scale: f(v, "m_fl4NoiseScale", 0.001),
        },
        "C_OP_RampScalarLinear" => Op::Ramp {
            field: i(v, "m_nField", F_ALPHA),
            rmin: f(v, "m_RateMin", 0.0),
            rmax: f(v, "m_RateMax", 0.0),
            t0: f(v, "m_flStartTime_min", 0.0),
            t1: f(v, "m_flEndTime_max", 1.0),
        },
        "C_OP_RampScalarLinearSimple" => {
            let r = f(v, "m_Rate", 0.0);
            Op::Ramp {
                field: i(v, "m_nField", F_ALPHA),
                rmin: r,
                rmax: r,
                t0: f(v, "m_flStartTime", 0.0),
                t1: f(v, "m_flEndTime", 1.0),
            }
        }
        "C_OP_MaxVelocity" => Op::MaxVelocity(f(v, "m_flMaxVelocity", 0.0)),
        "C_OP_OrientTo2dDirection" => Op::OrientTo2d(f(v, "m_flRotOffset", 0.0)),
        "C_OP_ClampScalar" => Op::Clamp {
            field: i(v, "m_nFieldOutput", F_RADIUS),
            min: f(v, "m_flOutputMin", 0.0),
            max: f(v, "m_flOutputMax", 1.0),
        },
        "C_OP_RandomForce" => Op::RandomForce {
            min: v3(v, "m_MinForce", [0.0; 3]),
            max: v3(v, "m_MaxForce", [0.0; 3]),
        },
        "C_OP_ExternalWindForce" => Op::Wind,
        "C_OP_AttractToControlPoint" => Op::Attract {
            amount: f(v, "m_fForceAmount", 100.0),
            power: f(v, "m_fFalloffPower", 0.0),
            offset: v3(v, "m_vecComponentScale", [1.0; 3]),
        },
        "C_OP_TwistAroundAxis" => Op::Twist {
            amount: f(v, "m_fForceAmount", 0.0),
            axis: v3(v, "m_TwistAxis", [0.0, 0.0, 1.0]),
            local: b(v, "m_bLocalSpace", false),
        },
        "C_OP_ConstrainDistance" => Op::Constrain {
            min: f(v, "m_fMinDistance", 0.0),
            max: f(v, "m_fMaxDistance", 100.0),
            offset: v3(v, "m_CenterOffset", [0.0; 3]),
        },
        "C_OP_PlanarConstraint" => Op::Plane {
            point: v3(v, "m_PointOnPlane", [0.0; 3]),
            normal: v3(v, "m_PlaneNormal", [0.0, 0.0, 1.0]),
        },
        _ => return None,
    })
}

fn parse_render(v: &Value, textures: &HashMap<String, usize>) -> Render {
    match s(v, "_class") {
        "C_OP_RenderSprites" => {
            let texture = v
                .get("m_vecTexturesInput")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|t| t.get("m_hTexture"))
                .and_then(|h| h.get("$resource"))
                .and_then(Value::as_str)
                .and_then(|r| textures.get(r).copied());
            let blend = match s(v, "m_nOutputBlendMode") {
                "PARTICLE_OUTPUT_BLEND_MODE_ADD" | "PARTICLE_OUTPUT_BLEND_MODE_BLEND_ADD"
                | "PARTICLE_OUTPUT_BLEND_MODE_HALF_BLEND_ADD" | "PARTICLE_OUTPUT_BLEND_MODE_NEG_HALF_BLEND_ADD" => Blend::Add,
                "PARTICLE_OUTPUT_BLEND_MODE_MOD2X" => Blend::Mod2x,
                _ => Blend::Alpha,
            };
            let orientation = s(v, "m_nOrientationType");
            Render::Sprite(Sprite {
                texture,
                blend,
                overbright: f(v, "m_flOverbrightFactor", 1.0),
                add_self: f(v, "m_flAddSelfAmount", 0.0),
                rate: f(v, "m_flAnimationRate", 0.1),
                fps: b(v, "m_bAnimateInFPS", false),
                fit: s(v, "m_nAnimationType") == "ANIMATION_TYPE_FIT_LIFETIME",
                flat: orientation == "PARTICLE_ORIENTATION_WORLD_Z_ALIGNED"
                    || orientation == "PARTICLE_ORIENTATION_ALIGN_TO_PARTICLE_NORMAL",
                min_size: f(v, "m_flMinSize", 0.0),
                max_size: f(v, "m_flMaxSize", 5000.0),
                fade_start: f(v, "m_flStartFadeSize", 1.0e8),
                fade_end: f(v, "m_flEndFadeSize", 2.0e8),
                alpha_scale: f(v, "m_flAlphaScale", 1.0),
                global_light: b(v, "m_bTintByGlobalLight", false),
                no_depth: b(v, "m_bDisableZBuffering", false),
            })
        }
        "C_OP_RenderModels" => {
            let stem = v
                .get("m_ModelList")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|m| m.get("m_model"))
                .and_then(|m| m.get("$resource"))
                .and_then(Value::as_str)
                .and_then(|r| Path::new(r).file_stem())
                .and_then(|n| n.to_str())
                .map(str::to_lowercase);
            match stem {
                Some(stem) => {
                    let rock = stem.starts_with("fallingrock");
                    Render::Model(ModelDef {
                        clip: if rock { "anim" } else { "idle" },
                        rate: (f(v, "m_flAnimationRate", 30.0) / 30.0).clamp(0.5, 6.0),
                        stem,
                    })
                }
                None => Render::None,
            }
        }
        "C_OP_RenderDeferredLight" => {
            let c = v
                .get("m_vecColorScale")
                .map(|c| color(c, "m_LiteralColor", [255.0; 4]))
                .unwrap_or([255.0; 4]);
            Render::Light(LightDef {
                radius_scale: f(v, "m_flRadiusScale", 1.0),
                alpha_scale: f(v, "m_flAlphaScale", 1.0),
                color: [c[0] / 255.0, c[1] / 255.0, c[2] / 255.0],
            })
        }
        _ => Render::None,
    }
}

fn parse_sheet(v: &Value) -> Sheet {
    let sequences = v
        .get("sheet")
        .and_then(Value::as_array)
        .map(|seqs| {
            seqs.iter()
                .map(|q| {
                    let mut t = 0.0;
                    let frames: Vec<(f32, [f32; 4])> = q
                        .get("frames")
                        .and_then(Value::as_array)
                        .map(|fr| {
                            fr.iter()
                                .filter_map(|fm| {
                                    let shown = fm.get("display_time").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                                    let img = fm.get("images")?.as_array()?.first()?;
                                    let r = img.get("uncropped").or_else(|| img.get("cropped"))?.as_array()?;
                                    let c = |k: usize| r.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                                    t += shown.max(1.0e-4);
                                    Some((t, [c(0), c(1), c(2), c(3)]))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    SheetSequence {
                        total: t.max(1.0e-4),
                        clamp: q.get("clamp").and_then(Value::as_bool).unwrap_or(false),
                        frames,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Sheet { sequences }
}

impl Sheet {
    fn uv(&self, sequence: f32, time: f32) -> [f32; 4] {
        if self.sequences.is_empty() {
            return [0.0, 0.0, 1.0, 1.0];
        }
        let q = &self.sequences[(sequence.max(0.0) as usize) % self.sequences.len()];
        if q.frames.is_empty() {
            return [0.0, 0.0, 1.0, 1.0];
        }
        let t = if q.clamp { time.clamp(0.0, q.total - 1.0e-4) } else { time.rem_euclid(q.total) };
        q.frames.iter().find(|fr| t < fr.0).map_or(q.frames[q.frames.len() - 1].1, |fr| fr.1)
    }
    fn total(&self, sequence: f32) -> f32 {
        if self.sequences.is_empty() {
            return 1.0;
        }
        self.sequences[(sequence.max(0.0) as usize) % self.sequences.len()].total
    }
}

fn parse_library(root: &Path) -> Option<Library> {
    let text = std::fs::read_to_string(root.join("particles.json")).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    let mut texture_index = HashMap::new();
    let mut textures = Vec::new();
    if let Some(t) = json.get("textures").and_then(Value::as_object) {
        for (name, info) in t {
            let file = info.get("file").and_then(Value::as_str).unwrap_or("");
            texture_index.insert(name.clone(), textures.len());
            textures.push((root.join(file), parse_sheet(info)));
        }
    }
    let defs = json.get("systems")?.as_object()?;
    let mut index = HashMap::new();
    for (k, name) in defs.keys().enumerate() {
        index.insert(name.clone(), k);
    }
    let mut systems = Vec::with_capacity(defs.len());
    for (name, sys) in defs {
        let d = sys.get("definition").unwrap_or(&Value::Null);
        let list = |key: &str| d.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        let enabled = |v: &Value| !b(v, "m_bDisableOperator", false);
        let mut ops: Vec<Op> = Vec::new();
        for key in ["m_Operators", "m_ForceGenerators", "m_Constraints"] {
            ops.extend(list(key).iter().filter(|v| enabled(v)).filter_map(parse_op));
        }
        let children = sys
            .get("children")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter(|ch| !ch.get("disabled").and_then(Value::as_bool).unwrap_or(false))
                    .filter(|ch| !ch.get("end_cap").and_then(Value::as_bool).unwrap_or(false))
                    .filter_map(|ch| {
                        let r = ch.get("ref")?.as_str()?;
                        Some((*index.get(r)?, ch.get("delay").and_then(Value::as_f64).unwrap_or(0.0) as f32))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let cps = list("m_PreEmissionOperators")
            .iter()
            .filter(|v| s(v, "_class") == "C_OP_SetSingleControlPointPosition")
            .map(|v| (i(v, "m_nCP1", 1).clamp(0, 7) as usize, v3(v, "m_vecCP1Pos", [128.0, 0.0, 0.0])))
            .collect();
        let restart = list("m_Operators")
            .iter()
            .find(|v| s(v, "_class") == "C_OP_RestartAfterDuration")
            .map(|v| (f(v, "m_flDurationMin", 1.0), f(v, "m_flDurationMax", 1.0)));
        systems.push(System {
            cps,
            restart,
            name: name.clone(),
            max: (match i(d, "m_nMaxParticles", 1000) {
                n if n <= 0 => 1000,
                n => n as usize,
            })
            .min(PARTICLE_CAP),
            radius: f(d, "m_flConstantRadius", 5.0),
            lifetime: f(d, "m_flConstantLifespan", 1.0),
            color: color(d, "m_ConstantColor", [255.0; 4]),
            emitters: list("m_Emitters").iter().filter(|v| enabled(v)).filter_map(parse_emitter).collect(),
            inits: list("m_Initializers").iter().filter(|v| enabled(v)).filter_map(parse_init).collect(),
            ops,
            renders: list("m_Renderers").iter().filter(|v| enabled(v)).map(|r| parse_render(r, &texture_index)).collect(),
            children,
            draw_distance: d
                .get("m_flMaxDrawDistance")
                .and_then(Value::as_f64)
                .map_or(DEFAULT_DRAW_DISTANCE, |x| (x as f32 * INCH).clamp(10.0, DEFAULT_DRAW_DISTANCE)),
        });
    }
    let instances = json
        .get("instances")?
        .as_array()?
        .iter()
        .filter(|p| p.get("start_active").and_then(Value::as_bool).unwrap_or(true))
        .filter_map(|p| {
            let system = *index.get(p.get("effect")?.as_str()?)?;
            let pos = v3(p, "pos", [0.0; 3]);
            let rows = p.get("rot")?.as_array()?;
            let mut rot = [[0.0; 3]; 3];
            for (r, row) in rows.iter().take(3).enumerate() {
                for (c, x) in row.as_array()?.iter().take(3).enumerate() {
                    rot[r][c] = x.as_f64()? as f32;
                }
            }
            let day = p.get("show_day").and_then(Value::as_bool).unwrap_or(true);
            let night = p.get("show_night").and_then(Value::as_bool).unwrap_or(true);
            // Both off (the candles): shown all the time.
            let (day, night) = if !day && !night { (true, true) } else { (day, night) };
            Some(Placement { system, pos, rot, day, night })
        })
        .collect();
    Some(Library { systems, textures, instances })
}

// -------------------------------------------------------------- simulation --

#[derive(Clone, Default)]
struct P {
    pos: V3,
    vel: V3,
    age: f32,
    life: f32,
    radius: f32,
    radius0: f32,
    roll: f32,
    roll_speed: f32,
    color: V3,
    color0: V3,
    fade_to: V3,
    alpha: f32,
    alpha0: f32,
    alpha2: f32,
    sequence: f32,
    yaw: f32,
    seed: f32,
    fade_in: f32,
    fade_out: f32,
    osc: [f32; 4],
    dead: bool,
}

/// One running system (an entity's root or one of its children).
struct Run {
    system: usize,
    parent: Option<usize>,
    restart_in: f32,
    delay: f32,
    time: f32,
    emitted: Vec<f32>,
    fired: Vec<bool>,
    particles: Vec<P>,
    rng: Rng,
}

impl Run {
    fn new(system: usize, delay: f32, seed: u64) -> Self {
        Run {
            system,
            parent: None,
            restart_in: 0.0,
            delay,
            time: 0.0,
            emitted: Vec::new(),
            fired: Vec::new(),
            particles: Vec::new(),
            rng: Rng::new(seed),
        }
    }
}

fn set_field(p: &mut P, field: i64, x: f32, scale: bool) {
    let slot = match field {
        F_LIFETIME => &mut p.life,
        F_RADIUS => &mut p.radius,
        F_ROLL => &mut p.roll,
        F_ROLL_SPEED => &mut p.roll_speed,
        F_ALPHA => &mut p.alpha,
        F_SEQUENCE => &mut p.sequence,
        F_YAW => &mut p.yaw,
        F_ALPHA2 => &mut p.alpha2,
        _ => return,
    };
    if scale {
        *slot *= x;
    } else {
        *slot = x;
    }
}

fn get_field(p: &P, field: i64) -> f32 {
    match field {
        F_LIFETIME => p.life,
        F_RADIUS => p.radius,
        F_ROLL => p.roll,
        F_ROLL_SPEED => p.roll_speed,
        F_ALPHA => p.alpha,
        F_SEQUENCE => p.sequence,
        F_YAW => p.yaw,
        F_ALPHA2 => p.alpha2,
        _ => 0.0,
    }
}

/// Smooth per-particle noise in -1..1 from a few sines.
fn wave(t: f32, seed: f32, k: f32) -> f32 {
    let a = (t * 1.7 * k + seed * 13.1).sin();
    let b = (t * 2.9 * k + seed * 7.3).sin();
    let c = (t * 4.3 * k + seed * 3.7).sin();
    (a * 0.5 + b * 0.3 + c * 0.2).clamp(-1.0, 1.0)
}

fn spawn(sys: &System, rng: &mut Rng, cps: &[V3; 8], parent: Option<&[P]>, rot: &[[f32; 3]; 3]) -> P {
    let mut p = P {
        life: sys.lifetime,
        radius: sys.radius,
        color: [sys.color[0], sys.color[1], sys.color[2]],
        alpha: sys.color[3] / 255.0,
        alpha2: 1.0,
        seed: rng.next(),
        ..Default::default()
    };
    for init in &sys.inits {
        match init {
            Init::Float { field, value, scale } => {
                let x = value.eval(&p, rng);
                set_field(&mut p, *field, x, *scale);
            }
            Init::Sphere { rmin, rmax, bias, smin, smax, lmin, lmax } => {
                let mut d;
                loop {
                    d = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
                    let l = len(d);
                    if l > 1.0e-3 && l <= 1.0 {
                        d = mul(d, 1.0 / l);
                        break;
                    }
                }
                let d = [d[0] * bias[0], d[1] * bias[1], d[2] * bias[2]];
                let r = rng.range(*rmin, *rmax);
                p.pos = add(p.pos, mul(d, r));
                let speed = rng.range(*smin, *smax);
                let local = rng.v3(*lmin, *lmax);
                p.vel = add(p.vel, add(mul(d, speed), orient(rot, local)));
            }
            Init::Offset { min, max, local } => {
                let o = rng.v3(*min, *max);
                p.pos = add(p.pos, if *local { orient(rot, o) } else { o });
            }
            Init::Color { min, max } => {
                let t = rng.next();
                p.color = [lerp(min[0], max[0], t), lerp(min[1], max[1], t), lerp(min[2], max[2], t)];
            }
            Init::Sequence { min, max } => p.sequence = rng.range(*min, *max + 0.999).floor(),
            Init::VelocityNoise { min, max, local } => {
                let o = rng.v3(*min, *max);
                p.vel = add(p.vel, if *local { orient(rot, o) } else { o });
            }
            Init::VelocityRandom { smin, smax, lmin, lmax } => {
                let d = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
                let l = len(d).max(1.0e-3);
                let local = rng.v3(*lmin, *lmax);
                p.vel = add(p.vel, add(mul(d, rng.range(*smin, *smax) / l), orient(rot, local)));
            }
            Init::Scalar { field, min, max } => {
                let x = rng.range(*min, *max);
                set_field(&mut p, *field, x, false);
            }
            Init::Path { end, bulge_cp, bulge, spread } => {
                let t = rng.next();
                let a = cps[0];
                let e = cps[*end];
                let mut q = add(a, mul(sub(e, a), t));
                let side = sub(cps[*bulge_cp], a);
                let arc = (std::f32::consts::PI * t).sin() * bulge * rng.range(-1.0, 1.0);
                q = add(q, mul(side, arc));
                let r = rng.range(0.0, *spread);
                let d = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
                p.pos = add(p.pos, add(q, mul(d, r / len(d).max(1.0e-3))));
            }
            Init::FromParent => {
                if let Some(parent) = parent.filter(|ps| !ps.is_empty()) {
                    let k = (rng.next() * parent.len() as f32) as usize % parent.len();
                    p.pos = add(p.pos, parent[k].pos);
                }
            }
        }
    }
    p.life = p.life.max(0.01);
    p.radius0 = p.radius;
    p.color0 = p.color;
    p.alpha0 = p.alpha;
    p.fade_to = p.color;
    p.osc = [rng.next(), rng.next(), rng.next(), rng.next()];
    p.fade_in = rng.next();
    p.fade_out = rng.next();
    for op in &sys.ops {
        if let Op::ColorToRandom { min, max, .. } = op {
            p.fade_to = lerp3(*min, *max, [rng.next(); 3]);
        }
    }
    p
}

/// A vector in the system's local space (control point 0's orientation, the
/// entity's angles) turned into Source world axes.
fn orient(rot: &[[f32; 3]; 3], v: V3) -> V3 {
    let o = [v[1], v[2], v[0]];
    let m = [
        rot[0][0] * o[0] + rot[0][1] * o[1] + rot[0][2] * o[2],
        rot[1][0] * o[0] + rot[1][1] * o[1] + rot[1][2] * o[2],
        rot[2][0] * o[0] + rot[2][1] * o[1] + rot[2][2] * o[2],
    ];
    [m[2], m[0], m[1]]
}

fn bias(x: f32, b: f32) -> f32 {
    // Schlick's bias: 0.5 leaves x alone.
    let b = b.clamp(0.001, 0.999);
    x / ((1.0 / b - 2.0) * (1.0 - x) + 1.0)
}

fn operate(sys: &System, p: &mut P, dt: f32, rng: &mut Rng, rot: &[[f32; 3]; 3]) {
    p.age += dt;
    let n = (p.age / p.life).clamp(0.0, 1.0);
    let mut alpha = p.alpha0;
    let mut radius = p.radius0;
    let mut decays = false;
    for op in &sys.ops {
        match op {
            Op::Decay => decays = true,
            Op::Movement { gravity, drag } => {
                p.vel = add(p.vel, mul(*gravity, dt));
                p.vel = mul(p.vel, (1.0 - drag.clamp(0.0, 1.0)).powf(dt * 30.0));
            }
            Op::Radius { t0, t1, s0, s1, bias: bb } => {
                let t = ((n - t0) / (t1 - t0).max(1.0e-4)).clamp(0.0, 1.0);
                radius *= lerp(*s0, *s1, bias(t, *bb));
            }
            Op::FadeInSimple(t) => alpha *= (n / t.max(1.0e-4)).clamp(0.0, 1.0),
            Op::FadeOutSimple(t) => alpha *= ((1.0 - n) / t.max(1.0e-4)).clamp(0.0, 1.0),
            Op::FadeIn { min, max, proportional } => {
                let t = lerp(*min, *max, p.fade_in);
                let x = if *proportional { n } else { p.age };
                alpha *= (x / t.max(1.0e-4)).clamp(0.0, 1.0);
            }
            Op::FadeOut { min, max, proportional } => {
                let t = lerp(*min, *max, p.fade_out);
                let x = if *proportional { 1.0 - n } else { p.life - p.age };
                let k = (x / t.max(1.0e-4)).clamp(0.0, 1.0);
                alpha *= k * k * (3.0 - 2.0 * k);
            }
            Op::FadeAndKill { a0, a1, in0, in1, out0, out1 } => {
                let k = if n < *in1 {
                    lerp(*a0, 1.0, ((n - in0) / (in1 - in0).max(1.0e-4)).clamp(0.0, 1.0))
                } else if n > *out0 {
                    lerp(1.0, *a1, ((n - out0) / (out1 - out0).max(1.0e-4)).clamp(0.0, 1.0))
                } else {
                    1.0
                };
                alpha *= k;
                decays = true;
            }
            Op::ColorTo { target, t0, t1 } => {
                let t = ((n - t0) / (t1 - t0).max(1.0e-4)).clamp(0.0, 1.0);
                p.color = lerp3(p.color0, *target, [t; 3]);
            }
            Op::ColorToRandom { t0, t1, .. } => {
                let t = ((n - t0) / (t1 - t0).max(1.0e-4)).clamp(0.0, 1.0);
                p.color = lerp3(p.color0, p.fade_to, [t; 3]);
            }
            Op::SpinUpdate => p.roll += p.roll_speed * dt,
            Op::Spin { rate, min } => {
                let r = rate.abs().max(*min).copysign(*rate);
                p.roll += r * dt;
            }
            Op::SetFloat { field, value, scale } => {
                let x = value.eval(p, rng);
                match (*field, *scale) {
                    (F_ALPHA, true) => alpha *= x,
                    (F_ALPHA, false) => alpha = x,
                    (F_RADIUS, true) => radius *= x,
                    (F_RADIUS, false) => radius = x,
                    (other, _) => set_field(p, other, x, false),
                }
            }
            Op::Oscillate { field, rmin, rmax, fmin, fmax, t0, t1, proportional } => {
                let x = if *proportional { n } else { p.age };
                if x >= *t0 && x <= *t1 {
                    let rate = lerp(*rmin, *rmax, p.osc[0]);
                    let freq = lerp(*fmin, *fmax, p.osc[1]);
                    let delta = rate * (std::f32::consts::TAU * freq * p.age + p.osc[2] * 6.283).sin() * dt;
                    match *field {
                        F_ALPHA => alpha *= (1.0 + delta * 4.0).clamp(0.0, 2.0),
                        F_RADIUS => radius *= (1.0 + delta * 4.0).clamp(0.2, 3.0),
                        other => {
                            let v = get_field(p, other) + delta;
                            set_field(p, other, v, false);
                        }
                    }
                }
            }
            Op::OscillateVector { rmin, rmax, fmin, fmax, t0, t1 } => {
                if n >= *t0 && n <= *t1 {
                    for a in 0..3 {
                        let rate = lerp(rmin[a], rmax[a], p.osc[a]);
                        let freq = lerp(fmin[a], fmax[a], p.osc[(a + 1) % 4]);
                        p.pos[a] += rate * (std::f32::consts::TAU * freq * p.age + p.osc[3] * 6.283).sin() * dt;
                    }
                }
            }
            Op::VectorNoise { min, max, scale, additive } => {
                let k = (scale * 100.0).clamp(0.2, 8.0);
                let w = [wave(p.age, p.seed, k), wave(p.age, p.seed + 0.37, k), wave(p.age, p.seed + 0.71, k)];
                let v = lerp3(*min, *max, mul(add(w, [1.0; 3]), 0.5));
                if *additive {
                    p.pos = add(p.pos, mul(v, dt * 30.0));
                } else {
                    p.pos = add(p.pos, mul(v, dt));
                }
            }
            Op::Noise { field, min, max, scale } => {
                let k = (scale * 1000.0).clamp(0.2, 8.0);
                let x = lerp(*min, *max, wave(p.age, p.seed, k) * 0.5 + 0.5);
                match *field {
                    F_ALPHA => alpha = x,
                    F_RADIUS => radius = x,
                    other => set_field(p, other, x, false),
                }
            }
            Op::Ramp { field, rmin, rmax, t0, t1 } => {
                if n >= *t0 && n <= *t1 {
                    let r = lerp(*rmin, *rmax, p.osc[3]);
                    match *field {
                        F_ALPHA => p.alpha0 = (p.alpha0 + r * dt).max(0.0),
                        F_RADIUS => p.radius0 = (p.radius0 + r * dt).max(0.0),
                        other => {
                            let v = get_field(p, other) + r * dt;
                            set_field(p, other, v, false);
                        }
                    }
                }
            }
            Op::MaxVelocity(m) => {
                let l = len(p.vel);
                if *m > 0.0 && l > *m {
                    p.vel = mul(p.vel, m / l);
                }
            }
            Op::OrientTo2d(offset) => {
                if p.vel[0].abs() + p.vel[1].abs() > 1.0e-3 {
                    p.roll = p.vel[1].atan2(p.vel[0]).to_degrees() + offset;
                }
            }
            Op::Clamp { field, min, max } => {
                let v = get_field(p, *field).clamp(*min, *max);
                set_field(p, *field, v, false);
            }
            Op::RandomForce { min, max } => p.vel = add(p.vel, mul(rng.v3(*min, *max), dt)),
            Op::Wind => {}
            Op::Attract { amount, power, .. } => {
                let d = len(p.pos).max(1.0);
                let pull = amount / d.powf(*power);
                p.vel = sub(p.vel, mul(p.pos, pull * dt / d));
            }
            Op::Twist { amount, axis, local } => {
                let axis = if *local { orient(rot, *axis) } else { *axis };
                let t = [axis[1] * p.pos[2] - axis[2] * p.pos[1], axis[2] * p.pos[0] - axis[0] * p.pos[2], axis[0] * p.pos[1] - axis[1] * p.pos[0]];
                let l = len(t);
                if l > 1.0e-3 {
                    p.vel = add(p.vel, mul(t, amount * dt / l));
                }
            }
            Op::Constrain { .. } | Op::Plane { .. } => {}
        }
    }
    p.pos = add(p.pos, mul(p.vel, dt));
    for op in &sys.ops {
        match op {
            Op::Constrain { min, max, offset } => {
                let d = sub(p.pos, *offset);
                let l = len(d);
                if l > *max && l > 1.0e-4 {
                    p.pos = add(*offset, mul(d, max / l));
                } else if l < *min && l > 1.0e-4 {
                    p.pos = add(*offset, mul(d, min / l));
                }
            }
            Op::Plane { point, normal } => {
                let d = (p.pos[0] - point[0]) * normal[0] + (p.pos[1] - point[1]) * normal[1] + (p.pos[2] - point[2]) * normal[2];
                if d < 0.0 {
                    p.pos = sub(p.pos, mul(*normal, d));
                }
            }
            _ => {}
        }
    }
    p.alpha = alpha.clamp(0.0, 4.0) * p.alpha2.clamp(0.0, 1.0);
    p.radius = radius.max(0.0);
    if decays && p.age >= p.life {
        p.dead = true;
    }
    if !decays && p.age >= p.life * 8.0 + 30.0 {
        p.dead = true;
    }
}

fn simulate(lib: &Library, run: &mut Run, dt: f32, cps: &[V3; 8], parent: Option<&[P]>, rot: &[[f32; 3]; 3]) {
    if run.delay > 0.0 {
        run.delay -= dt;
        return;
    }
    let sys = &lib.systems[run.system];
    run.time += dt;
    if run.emitted.len() != sys.emitters.len() {
        run.emitted = vec![0.0; sys.emitters.len()];
        run.fired = vec![false; sys.emitters.len()];
    }
    let dummy = P::default();
    let mut new = 0usize;
    for (k, e) in sys.emitters.iter().enumerate() {
        match e {
            Emitter::Continuous { rate, start, duration } => {
                if run.time >= *start && (*duration <= 0.0 || run.time <= start + duration) {
                    let r = rate.eval(&dummy, &mut run.rng).max(0.0);
                    run.emitted[k] += r * dt;
                    let whole = run.emitted[k].floor();
                    run.emitted[k] -= whole;
                    new += whole as usize;
                }
            }
            Emitter::Instant { count, start } => {
                if !run.fired[k] && run.time >= *start {
                    run.fired[k] = true;
                    new += count.eval(&dummy, &mut run.rng).max(0.0) as usize;
                }
            }
        }
    }
    let room = sys.max.saturating_sub(run.particles.len());
    for _ in 0..new.min(room) {
        let p = spawn(sys, &mut run.rng, cps, parent, rot);
        run.particles.push(p);
    }
    for p in &mut run.particles {
        operate(sys, p, dt, &mut run.rng, rot);
    }
    run.particles.retain(|p| !p.dead);
    // Systems that only burst once start over when everything has died, the
    // way the map's looping splashes are retriggered.
    if run.particles.is_empty()
        && !sys.emitters.is_empty()
        && run.fired.iter().zip(&sys.emitters).all(|(f, e)| !matches!(e, Emitter::Instant { .. }) || *f)
        && sys.emitters.iter().all(|e| matches!(e, Emitter::Instant { .. }))
        && run.time > 0.5
    {
        run.time = 0.0;
        run.fired.iter_mut().for_each(|f| *f = false);
    }
}

struct Live {
    placement: usize,
    runs: Vec<Run>,
    cps: [V3; 8],
    rot: [[f32; 3]; 3],
}

impl Live {
    fn new(lib: &Library, placement: usize) -> Live {
        let mut runs = Vec::new();
        let mut seed = placement as u64 * 977 + 13;
        World::runs_for(lib, lib.instances[placement].system, 0.0, None, &mut seed, &mut runs);
        let mut cps = [[0.0; 3]; 8];
        let mut set = [false; 8];
        set[0] = true;
        for r in &runs {
            for (k, v) in &lib.systems[r.system].cps {
                if !set[*k] {
                    cps[*k] = orient(&lib.instances[placement].rot, *v);
                    set[*k] = true;
                }
            }
        }
        // Unset points sit a little ahead, so paths have a direction.
        for k in 1..8 {
            if !set[k] {
                cps[k] = orient(&lib.instances[placement].rot, [128.0, 0.0, 0.0]);
            }
        }
        Live { placement, runs, cps, rot: lib.instances[placement].rot }
    }

    fn step(&mut self, lib: &Library, dt: f32) {
        let mut restart = false;
        for r in &mut self.runs {
            if let Some((lo, hi)) = lib.systems[r.system].restart {
                r.restart_in -= dt;
                if r.restart_in <= 0.0 {
                    r.restart_in = r.rng.range(lo, hi.max(lo));
                    restart = true;
                }
            }
        }
        if restart {
            for r in &mut self.runs {
                if lib.systems[r.system].restart.is_none() {
                    r.time = 0.0;
                    r.fired.iter_mut().for_each(|x| *x = false);
                    r.emitted.iter_mut().for_each(|x| *x = 0.0);
                }
            }
        }
        for k in 0..self.runs.len() {
            let (head, tail) = self.runs.split_at_mut(k);
            let run = &mut tail[0];
            let parent = run.parent.and_then(|p| head.get(p)).map(|p| p.particles.as_slice());
            simulate(lib, run, dt, &self.cps, parent, &self.rot);
        }
    }
}

struct World {
    lib: Library,
    live: Vec<Live>,
    clock: Option<Instant>,
    debt: f32,
}

impl World {
    fn runs_for(lib: &Library, system: usize, delay: f32, parent: Option<usize>, seed: &mut u64, out: &mut Vec<Run>) {
        *seed += 1;
        let mut run = Run::new(system, delay, *seed);
        run.parent = parent;
        let me = out.len();
        out.push(run);
        if out.len() > 32 {
            return;
        }
        for (child, d) in lib.systems[system].children.clone() {
            World::runs_for(lib, child, delay + d, Some(me), seed, out);
        }
    }

    fn update(&mut self, camera: [f32; 3], day: f32) {
        let now = Instant::now();
        let elapsed = self.clock.map_or(0.0, |c| now.duration_since(c).as_secs_f32()).min(0.25);
        self.clock = Some(now);
        self.debt += elapsed;
        let is_day = day >= 0.5;
        let lib = &self.lib;
        let near = |k: usize| {
            let pl = &lib.instances[k];
            let d = len(sub(pl.pos, camera));
            d < lib.systems[pl.system].draw_distance && if is_day { pl.day } else { pl.night }
        };
        self.live.retain(|l| near(l.placement));
        let have: std::collections::HashSet<usize> = self.live.iter().map(|l| l.placement).collect();
        for k in 0..lib.instances.len() {
            if !have.contains(&k) && near(k) {
                let mut l = Live::new(lib, k);
                // Warm up so a system walked up to is already in full swing.
                for _ in 0..90 {
                    l.step(lib, 1.0 / 30.0);
                }
                self.live.push(l);
            }
        }
        let mut steps = 0;
        while self.debt >= STEP && steps < 8 {
            self.debt -= STEP;
            steps += 1;
            for l in &mut self.live {
                l.step(lib, STEP);
            }
        }
        if steps == 8 {
            self.debt = 0.0;
        }
    }

    /// A particle position (Source world axes, inches from control point 0)
    /// in map space.
    fn to_map(pl: &Placement, p: V3) -> V3 {
        [pl.pos[0] + p[1] * INCH, pl.pos[1] + p[2] * INCH, pl.pos[2] + p[0] * INCH]
    }
}

fn root() -> Option<PathBuf> {
    Some(crate::dota_map::data_root()?.join("particles"))
}

// ------------------------------------------------------------------ public --

/// A point light a particle system gives off: position (map metres), radius
/// (metres) and colour (linear, intensity included).
pub fn lights() -> Vec<([f32; 3], f32, [f32; 3])> {
    let Some(w) = state() else {
        return Vec::new();
    };
    let Ok(w) = w.lock() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for l in &w.live {
        let pl = &w.lib.instances[l.placement];
        for r in &l.runs {
            let sys = &w.lib.systems[r.system];
            for render in &sys.renders {
                let Render::Light(def) = render else {
                    continue;
                };
                for p in &r.particles {
                    let a = p.alpha * def.alpha_scale;
                    if a <= 0.001 {
                        continue;
                    }
                    let c = [p.color[0] / 255.0 * def.color[0], p.color[1] / 255.0 * def.color[1], p.color[2] / 255.0 * def.color[2]];
                    out.push((World::to_map(pl, p.pos), p.radius * def.radius_scale * INCH, mul(c, a)));
                }
            }
        }
    }
    out
}

fn state() -> Option<&'static Mutex<World>> {
    static STATE: OnceLock<Mutex<World>> = OnceLock::new();
    static LOADING: AtomicBool = AtomicBool::new(false);
    static SLOT: Mutex<Option<Library>> = Mutex::new(None);
    if let Some(s) = STATE.get() {
        return Some(s);
    }
    if !LOADING.swap(true, Ordering::Relaxed) {
        std::thread::Builder::new()
            .name("dota-fx-load".into())
            .spawn(|| {
                let lib = root().and_then(|r| parse_library(&r));
                match &lib {
                    Some(l) => log::info!(
                        "Dota fx: {} systems, {} placements, {} textures",
                        l.systems.len(),
                        l.instances.len(),
                        l.textures.len()
                    ),
                    None => log::warn!("Dota fx: no particles.json"),
                }
                if let Ok(mut slot) = SLOT.lock() {
                    *slot = lib;
                }
            })
            .ok();
        return None;
    }
    let lib = SLOT.lock().ok()?.take()?;
    Some(STATE.get_or_init(|| Mutex::new(World { lib, live: Vec::new(), clock: None, debt: 0.0 })))
}

// --------------------------------------------------------------------- gpu --

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Quad {
    pos: [f32; 3],
    radius: f32,
    color: [f32; 4],
    uv: [f32; 4],
    /// roll, min size, max size, add self
    params: [f32; 4],
    /// fade start, fade end, flat (1), unused
    fade: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    /// forward xyz, focal length (NDC units at 1 m)
    forward: [f32; 4],
    /// exposure, fog start, fog end, unused
    misc: [f32; 4],
    fog_color: [f32; 4],
}

struct Gpu {
    pipelines: [wgpu::RenderPipeline; 4],
    uniform: wgpu::Buffer,
    groups: Vec<Option<wgpu::BindGroup>>,
    white: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    quads: Mutex<(wgpu::Buffer, usize)>,
}

const BLEND_PREMULTIPLIED: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};
const BLEND_ADD: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};
const BLEND_MOD2X: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Dst,
        dst_factor: wgpu::BlendFactor::Src,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

impl Gpu {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, lib: &Library) -> Gpu {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dota fx"),
            source: wgpu::ShaderSource::Wgsl(include_str!("dota_fx.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dota fx"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dota fx"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4
        ];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Quad>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attributes,
        })];
        let pipeline_vs = |blend: wgpu::BlendState, vs: &str, fs: &str, depth: bool, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(if depth { wgpu::CompareFunction::GreaterEqual } else { wgpu::CompareFunction::Always }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = |blend: wgpu::BlendState, fs: &str, depth: bool, label: &str| {
            pipeline_vs(blend, "vs", fs, depth, label)
        };
        let pipelines = [
            pipeline(BLEND_PREMULTIPLIED, "fs_alpha", true, "Dota fx alpha"),
            pipeline(BLEND_ADD, "fs_add", true, "Dota fx add"),
            pipeline(BLEND_MOD2X, "fs_mod2x", true, "Dota fx mod2x"),
            pipeline(BLEND_ADD, "fs_add", false, "Dota fx add on top"),
        ];
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dota fx uniform"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dota fx"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let group = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Dota fx"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            })
        };
        let groups = lib
            .textures
            .iter()
            .map(|(path, _)| {
                let (w, h, pixels, _) = crate::dota_map::load_rgba(path)?;
                Some(group(&crate::dota_map::image_texture(device, queue, w, h, &pixels)))
            })
            .collect();
        let white = group(&crate::dota_map::image_texture(device, queue, 1, 1, &[255, 255, 255, 255]));
        let quads = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dota fx quads"),
            size: (std::mem::size_of::<Quad>() * 4096) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Gpu { pipelines, uniform, groups, white, layout, sampler, quads: Mutex::new((quads, 4096)) }
    }
}

/// Simulate and draw the world particles near `camera`, inside the map pass
/// (reverse-Z depth, tested but not written). `exposure` and `fog_color` are
/// the map's, so the sprites sit in the same tonemapped image.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pass: &mut wgpu::RenderPass<'_>,
    format: wgpu::TextureFormat,
    camera: [f32; 3],
    view_proj: glam::Mat4,
    day: f32,
    exposure: f32,
    fog_color: [f32; 4],
    global_light: [f32; 3],
) {
    let Some(state) = state() else {
        return;
    };
    let Ok(mut w) = state.lock() else {
        return;
    };
    w.update(camera, day);
    static GPU: OnceLock<Gpu> = OnceLock::new();
    let gpu = GPU.get_or_init(|| Gpu::new(device, queue, format, &w.lib));
    // Camera basis from the inverse view-projection.
    let inv = view_proj.inverse();
    let at = |x: f32, y: f32, z: f32| inv.project_point3(glam::Vec3::new(x, y, z));
    // view_proj is camera-relative: the camera sits at the origin.
    let far = at(0.0, 0.0, 0.001);
    let forward = far.normalize_or_zero();
    let mid = at(0.0, 0.0, 0.5);
    let right = (at(1.0, 0.0, 0.5) - mid).normalize_or_zero();
    let up = (at(0.0, 1.0, 0.5) - mid).normalize_or_zero();
    let probe = view_proj.project_point3(forward + up);
    let focal = probe.y.abs().max(1.0e-3);
    let uniform = Uniform {
        view_proj: view_proj.to_cols_array_2d(),
        camera: [camera[0], camera[1], camera[2], 0.0],
        right: [right.x, right.y, right.z, 0.0],
        up: [up.x, up.y, up.z, 0.0],
        forward: [forward.x, forward.y, forward.z, focal],
        misc: [exposure, 25.0, 140.0, 0.0],
        fog_color,
    };
    queue.write_buffer(&gpu.uniform, 0, bytemuck::bytes_of(&uniform));
    // Quads bucketed by pipeline and texture.
    let mut buckets: HashMap<(usize, Option<usize>), Vec<Quad>> = HashMap::new();
    let lib = &w.lib;
    let mut creatures = Vec::new();
    for l in &w.live {
        let pl = &lib.instances[l.placement];
        for r in &l.runs {
            for render in &lib.systems[r.system].renders {
                let Render::Model(def) = render else {
                    continue;
                };
                for p in &r.particles {
                    if p.alpha <= 0.01 {
                        continue;
                    }
                    let (hx, hz) = (p.vel[1], p.vel[0]);
                    let yaw = if hx.abs() + hz.abs() > 1.0e-3 { hx.atan2(hz) } else { p.yaw.to_radians() };
                    let scale = p.radius.clamp(0.15, 4.0);
                    creatures.push(crate::dota_anim::Creature {
                        model: def.stem.clone(),
                        clip: def.clip.to_string(),
                        time: p.age * def.rate + p.seed * 7.0,
                        matrix: glam::Mat4::from_translation(glam::Vec3::from(World::to_map(pl, p.pos)))
                            * glam::Mat4::from_rotation_y(yaw)
                            * glam::Mat4::from_scale(glam::Vec3::splat(scale)),
                        tint: [1.0; 3],
                        fade: p.alpha.min(1.0),
                    });
                }
            }
        }
    }
    crate::dota_anim::set_creatures(creatures);
    for l in &w.live {
        let pl = &lib.instances[l.placement];
        for r in &l.runs {
            let sys = &lib.systems[r.system];
            for render in &sys.renders {
                let Render::Sprite(sp) = render else {
                    continue;
                };
                let pipeline = match (sp.blend, sp.no_depth) {
                    (Blend::Alpha, _) => 0,
                    (Blend::Add, false) => 1,
                    (Blend::Add, true) => 3,
                    (Blend::Mod2x, _) => 2,
                };
                let sheet = sp.texture.map(|t| &lib.textures[t].1);
                let tint = if sp.global_light { global_light } else { [1.0; 3] };
                let list = buckets.entry((pipeline, sp.texture)).or_default();
                for p in &r.particles {
                    let a = p.alpha * sp.alpha_scale;
                    if a <= 0.002 || p.radius <= 0.0 {
                        continue;
                    }
                    let uv = sheet.map_or([0.0, 0.0, 1.0, 1.0], |s| {
                        let t = if sp.fit {
                            p.age / p.life * s.total(p.sequence)
                        } else {
                            p.age * sp.rate
                        };
                        s.uv(p.sequence, t)
                    });
                    let srgb = |c: f32| {
                        let c = (c / 255.0).clamp(0.0, 1.0);
                        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
                    };
                    let k = sp.overbright;
                    list.push(Quad {
                        pos: World::to_map(pl, p.pos),
                        radius: p.radius * INCH,
                        color: [srgb(p.color[0]) * k * tint[0], srgb(p.color[1]) * k * tint[1], srgb(p.color[2]) * k * tint[2], a.min(1.0)],
                        uv,
                        params: [p.roll.to_radians(), sp.min_size, sp.max_size, sp.add_self],
                        fade: [sp.fade_start, sp.fade_end, if sp.flat { 1.0 } else { 0.0 }, p.yaw.to_radians()],
                    });
                }
            }
        }
    }
    let total: usize = buckets.values().map(Vec::len).sum();
    if total == 0 {
        return;
    }
    let Ok(mut quads) = gpu.quads.lock() else {
        return;
    };
    if total > quads.1 {
        let n = total.next_power_of_two();
        quads.0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dota fx quads"),
            size: (std::mem::size_of::<Quad>() * n) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        quads.1 = n;
    }
    let mut order: Vec<_> = buckets.into_iter().collect();
    // Alpha-blended sprites first, the additive glow on top, mod2x last.
    order.sort_by_key(|((pipeline, texture), _)| (*pipeline, texture.unwrap_or(usize::MAX)));
    let mut all = Vec::with_capacity(total);
    let mut ranges = Vec::new();
    for ((pipeline, texture), list) in order {
        let start = all.len() as u32;
        all.extend(list);
        ranges.push((pipeline, texture, start, all.len() as u32));
    }
    queue.write_buffer(&quads.0, 0, bytemuck::cast_slice(&all));
    pass.set_vertex_buffer(0, quads.0.slice(..));
    for (pipeline, texture, start, end) in ranges {
        pass.set_pipeline(&gpu.pipelines[pipeline]);
        let group = texture.and_then(|t| gpu.groups.get(t)).and_then(Option::as_ref).unwrap_or(&gpu.white);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..6, start..end);
    }
    let _ = (&gpu.layout, &gpu.sampler);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every effect placed on the map, run for a few seconds on its own:
    /// particle counts, sizes and alpha stay sane (`--ignored`, needs the data).
    #[test]
    #[ignore]
    fn map_effects_run() {
        let lib = parse_library(&root().expect("data")).expect("particles.json");
        let mut seen = std::collections::HashSet::new();
        for pl in &lib.instances {
            if !seen.insert(pl.system) {
                continue;
            }
            let k = lib.instances.iter().position(|q| q.system == pl.system).unwrap_or(0);
            let mut live = Live::new(&lib, k);
            for _ in 0..(5.0 / STEP) as usize {
                live.step(&lib, STEP);
            }
            let runs = live.runs;
            let names: Vec<&String> = Vec::new();
            let _ = names;
            for r in &runs {
                let sys = &lib.systems[r.system];
                let n = r.particles.len();
                let (mut rad, mut a, mut far) = (0.0, 0.0, 0.0_f32);
                for p in &r.particles {
                    rad += p.radius;
                    a += p.alpha;
                    far = far.max(len(p.pos));
                }
                let k = n.max(1) as f32;
                let kind = sys.renders.iter().map(|r| match r {
                    Render::Sprite(s) => format!("sprite {:?} tex {:?} max {}", s.blend, s.texture, s.max_size),
                    Render::Light(_) => "light".to_string(),
                    Render::Model(m) => format!("model {}", m.stem),
                    Render::None => "-".to_string(),
                }).collect::<Vec<_>>().join(",");
                println!(
                    "{:44} n {:4} radius {:7.2}in alpha {:5.2} spread {:7.1}in  {}",
                    sys.name.rsplit('/').next().unwrap_or(""), n, rad / k, a / k, far, kind
                );
            }
            println!("---");
        }
    }
}
