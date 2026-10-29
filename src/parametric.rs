//! Parametric-chain engine (API v14): IFC 4.3.2.0 alignment species
//! evaluation, chain audits and reference-only display staging builders.
//!
//! The record layer's DECLARED parameters (species enum, start point and
//! direction, signed radii, gradients, length) are the only truth; this
//! module turns them into plane geometry on demand. The species physics
//! follow the IFC 4.3.2.0 base formulas on the
//! `IfcAlignmentHorizontalSegmentTypeEnum` and
//! `IfcAlignmentVerticalSegmentTypeEnum` lexics pages: every transition
//! species is a curvature law between the segment's end curvatures and the
//! species differ only in the blend shape. With s = arc position,
//! L = segment length, xi = s/L and kappa1/kappa2 = signed end curvatures
//! (dkappa = kappa2 - kappa1):
//!
//! | Species | Plane | Law (IFC 4.3.2.0 lexics) | Carrier |
//! |---|---|---|---|
//! | LINE | H | kappa = 0 | `Line` (exact) |
//! | CIRCULARARC | H | kappa = 1/R over the arc | `Arc` (exact) |
//! | CLOTHOID | H | kappa(s) = kappa1 + xi*dkappa | chords (declared tolerance) |
//! | BLOSSCURVE | H | kappa(s) = kappa1 + (3-2*xi)*xi^2*dkappa | chords |
//! | COSINECURVE | H | kappa(s) = kappa1 + (1-cos(pi*xi))/2*dkappa | chords |
//! | CUBIC | H | y = x^3/(6*R*L) | `Spline` degree 3 (exact) |
//! | SINECURVE | H | kappa(s) = kappa1 + (xi - sin(2*pi*xi)/(2*pi))*dkappa | chords |
//! | HELMERTCURVE | H | two quadratic halves | chords |
//! | VIENNESEBEND | H | xi^4 quintic blend + bank correction | chords |
//! | CONSTANTGRADIENT | V | constant gradient | `Line` (exact) |
//! | CIRCULARARC | V | the exact circle in the (run, elevation) plane | `Arc` (exact) |
//! | PARABOLICARC | V | gradient linear in the run: g(u) = g1 + u/R | `Spline` degree 2 (exact) |
//! | CLOTHOID | V | kappa_v(s) = kappa_v1 + xi*dkappa_v | chords |
//!
//! Evaluation rules:
//!
//! - the tangent direction integrates the curvature law in closed form;
//! - the position integrates `(cos theta, sin theta)` by composite
//!   16-node Gauss-Legendre, sub-intervals following the total tangent
//!   deflection — one deterministic integrator for every transcendental
//!   species (road and rail ranges evaluate in one to five sub-intervals);
//! - the vertical circle is the exact circle in the (run, elevation) plane,
//!   sin(theta(u)) = sin(theta0) + u/R, h(u) = h0 + R*(cos(theta0) -
//!   cos(theta(u))); the vertical parabola h(u) = h0 + g0*u + u^2/(2*R)
//!   realizes the record model's mean-gradient law h(L) = h0 + L*(g1+g2)/2
//!   exactly;
//! - every IFC blend law integrates to half the curvature change over the
//!   segment (W(1) = 1/2), so a transition deflects through L/(2*R) exactly
//!   like the clothoid law; a Declared Piecewise table's deflection is its
//!   OWN integral W(1) — only the endpoint VALUE is the declared k1.
//!
//! Chain discipline: the declared data is validated BEFORE any geometry is
//! built (finite fields, non-negative lengths, radii present where the
//! species needs them, IFC self-consistency of constants such as the
//! parabolic vertex radius R = L/(g2-g1)); chains are audited seam-to-seam
//! against the record model's continuity tolerances (points 1e-6 m,
//! directions 1e-9 rad, gradients 1e-6) and refused with the audited pair
//! named; a chain that would stage more display entities than
//! `MAX_PLUGIN_LIVE_ENTITIES` publishes nothing. A HELIX entity is never
//! produced: the codec's Helix is a 3D constant-pitch space spiral, while a
//! plan clothoid is a 2D curve with linear curvature — staging one for the
//! other would be mathematically wrong.

use std::f64::consts::PI;

use codec::entities::{Arc, EntityType, Line, LwPolyline, LwVertex, Spline};
use codec::types::{Vector2, Vector3};
use ocs_plugin_api::host::{
    DeclaredKappaRamp, HorizontalSegmentSpecies, MAX_PLUGIN_LIVE_ENTITIES, ParametricChain,
    ParametricPlane, ParametricSample, ParametricSegment, ParametricSpecies,
    VerticalSegmentSpecies,
};

const TWO_PI: f64 = std::f64::consts::TAU;

/// Seam continuity tolerance for points, in meters: the record model's own
/// alignment point tolerance.
pub const SEAM_POINT_EPS: f64 = 1.0e-6;
/// Seam continuity tolerance for directions, in radians: the record model's
/// own alignment direction tolerance.
pub const SEAM_DIRECTION_EPS: f64 = 1.0e-9;
/// Seam continuity tolerance for gradients, in decimal fractions: the
/// record model's own alignment gradient tolerance.
pub const SEAM_GRADIENT_EPS: f64 = 1.0e-6;

/// Upper chord count per segment: once the refinement loop would exceed
/// this, the tolerance is judged unattainable and staging refuses instead
/// of looping.
const MAX_CHORDS_PER_SEGMENT: usize = 50_000;

/// Normalize a start direction into [0, 2*pi). Declared directions are
/// +-2*pi-legal (any winding is accepted by the record layer); the engine
/// normalizes once at the segment boundary.
fn normalize_angle(angle: f64) -> f64 {
    angle.rem_euclid(TWO_PI)
}

/// Unsigned angular distance between two (possibly differently wound)
/// angles.
fn angle_distance(a: f64, b: f64) -> f64 {
    let d = (normalize_angle(a) - normalize_angle(b)).rem_euclid(TWO_PI);
    if d > PI {
        TWO_PI - d
    } else {
        d
    }
}

/// Validated derived data of one declared segment.
#[derive(Debug, Clone, Copy)]
struct Vitals {
    /// Declared length: arc length for plan transitions, the horizontal run
    /// for CUBIC and every profile species (the record model's reading).
    length: f64,
    /// Plan: bearing in [0, 2*pi). Profile: slope angle atan(start_gradient).
    theta_start: f64,
    /// Signed start curvature kappa1 (1/R1; 0 for a flat end).
    kappa_start: f64,
    /// Signed end curvature kappa2 (0 = flat end).
    kappa_end: f64,
    /// Signed radius carried by arc/parabola species; 0 = none.
    radius: f64,
    /// Bank-angle change (radians) of the Viennese gravity-center term.
    bank_change: f64,
    /// Gravity-centre line height of the Viennese correction.
    gravity_height: f64,
}

/// Validate one declared segment per its species and derive the evaluation
/// vitals. Every refusal names the species and the offending field.
fn validate_segment(plane: ParametricPlane, segment: &ParametricSegment) -> Result<Vitals, String> {
    let fields = [
        segment.start_point[0],
        segment.start_point[1],
        segment.start_direction,
        segment.start_gradient,
        segment.end_gradient,
        segment.length,
        segment.start_radius,
        segment.end_radius,
        segment.end_curvature,
        segment.bank_change,
        segment.gravity_height,
    ];
    if !fields.iter().all(|f| f.is_finite()) {
        return Err(format!(
            "segment species {:?}: a declared field is not finite",
            segment.species
        ));
    }
    if segment.length < 0.0 {
        return Err(format!(
            "segment species {:?}: length is negative",
            segment.species
        ));
    }
    let plane_matches = match segment.species {
        ParametricSpecies::Horizontal(_) => plane == ParametricPlane::Plan,
        ParametricSpecies::Vertical(_) => plane == ParametricPlane::Profile,
    };
    if !plane_matches {
        return Err(format!(
            "segment species {:?} does not belong to the {:?} chain",
            segment.species, plane
        ));
    }
    let mut vitals = Vitals {
        length: segment.length,
        theta_start: normalize_angle(segment.start_direction),
        kappa_start: 0.0,
        kappa_end: 0.0,
        radius: 0.0,
        bank_change: 0.0,
        gravity_height: 0.0,
    };
    match segment.species {
        ParametricSpecies::Horizontal(species) => match species {
            HorizontalSegmentSpecies::Line => {}
            HorizontalSegmentSpecies::CircularArc => {
                if segment.start_radius == 0.0 {
                    return Err(
                        "plan circular arc requires a declared (non-zero) radius".to_owned()
                    );
                }
                if segment.end_radius != 0.0 && segment.end_radius != segment.start_radius {
                    return Err(format!(
                        "plan circular arc: start and end radii disagree ({} vs {})",
                        segment.start_radius, segment.end_radius
                    ));
                }
                let sweep = segment.length / segment.start_radius.abs();
                if sweep >= TWO_PI {
                    return Err(format!(
                        "plan circular arc sweeps {sweep} rad: a segment cannot sweep a full turn or more"
                    ));
                }
                vitals.radius = segment.start_radius;
                vitals.kappa_start = 1.0 / segment.start_radius;
                vitals.kappa_end = vitals.kappa_start;
            }
            HorizontalSegmentSpecies::Cubic => {
                if segment.end_radius == 0.0 {
                    return Err("plan cubic parabola requires a declared end radius".to_owned());
                }
                if segment.start_radius != 0.0 {
                    return Err(
                        "the cubic parabola starts flat; declare CLOTHOID for a curved start"
                            .to_owned(),
                    );
                }
                vitals.kappa_end = 1.0 / segment.end_radius;
            }
            HorizontalSegmentSpecies::BlossCurve
            | HorizontalSegmentSpecies::Clothoid
            | HorizontalSegmentSpecies::CosineCurve
            | HorizontalSegmentSpecies::HelmertCurve
            | HorizontalSegmentSpecies::SineCurve
            | HorizontalSegmentSpecies::VienneseBend => {
                if segment.end_radius == 0.0 {
                    return Err(format!(
                        "plan {species:?} requires a declared (non-zero) end radius"
                    ));
                }
                vitals.kappa_start = if segment.start_radius == 0.0 {
                    0.0
                } else {
                    1.0 / segment.start_radius
                };
                vitals.kappa_end = 1.0 / segment.end_radius;
                if is_banked_bend(&species) {
                    vitals.bank_change = segment.bank_change;
                    vitals.gravity_height = segment.gravity_height;
                }
            }
            HorizontalSegmentSpecies::Declared(ramp) => {
                // The declared law states kappa directly: k0 =
                // 1/start_radius (0 = a straight entry) and k1 =
                // end_curvature — a Declared blend may end flat (k1 = 0),
                // unlike the IFC blends, which always exit into a circle;
                // end_radius is ignored for a Declared law.
                validate_declared_ramp(&ramp).map_err(|reason| {
                    format!("plan declared blend: {reason}")
                })?;
                vitals.kappa_start = if segment.start_radius == 0.0 {
                    0.0
                } else {
                    1.0 / segment.start_radius
                };
                vitals.kappa_end = segment.end_curvature;
                if matches!(ramp, DeclaredKappaRamp::Viennese) {
                    vitals.bank_change = segment.bank_change;
                    vitals.gravity_height = segment.gravity_height;
                }
            }
        },
        ParametricSpecies::Vertical(species) => match species {
            VerticalSegmentSpecies::ConstantGradient => {
                // IFC 4.3.2.0 lexics: for CONSTANTGRADIENT the end gradient
                // is the same value as the start gradient.
                if (segment.end_gradient - segment.start_gradient).abs() > SEAM_GRADIENT_EPS {
                    return Err(format!(
                        "vertical constant gradient: declared end gradient {} differs from start {}",
                        segment.end_gradient, segment.start_gradient
                    ));
                }
                vitals.theta_start = segment.start_gradient.atan();
            }
            VerticalSegmentSpecies::ParabolicArc => {
                // The record model's signed relation R = L/(g2-g1) is the
                // IFC reading of the parabolic vertex radius.
                if segment.start_radius == 0.0 {
                    return Err(
                        "vertical parabolic arc requires a (non-zero) vertex radius".to_owned()
                    );
                }
                if (segment.length / segment.start_radius
                    - (segment.end_gradient - segment.start_gradient))
                .abs()
                    > SEAM_GRADIENT_EPS
                {
                    return Err(format!(
                        "vertical parabolic arc: vertex radius {} disagrees with gradients {}..{} over run {}",
                        segment.start_radius,
                        segment.start_gradient,
                        segment.end_gradient,
                        segment.length
                    ));
                }
                vitals.radius = segment.start_radius;
                vitals.theta_start = segment.start_gradient.atan();
            }
            VerticalSegmentSpecies::CircularArc => {
                if segment.start_radius == 0.0 {
                    return Err("vertical circular arc requires a (non-zero) radius".to_owned());
                }
                // The circle's exact run law must realize the declared end
                // gradient: the record carries the gradient pair per
                // segment, so an inconsistency here means the declared
                // radius disagrees with them.
                let theta0 = segment.start_gradient.atan();
                let s1 = theta0.sin() + segment.length / segment.start_radius;
                if !(s1 > -1.0 && s1 < 1.0) {
                    return Err(format!(
                        "vertical circular arc: run {} with radius {} leaves the sloped range",
                        segment.length, segment.start_radius
                    ));
                }
                let realized_gradient = s1.asin().tan();
                let declared = segment.end_gradient;
                if (realized_gradient - declared).abs()
                    > SEAM_GRADIENT_EPS * (1.0 + declared.abs() * declared.abs())
                {
                    return Err(format!(
                        "vertical circular arc: radius {} realizes end gradient {realized_gradient:.9} but the record declares {declared}",
                        segment.start_radius
                    ));
                }
                vitals.radius = segment.start_radius;
                vitals.theta_start = theta0;
            }
            VerticalSegmentSpecies::Clothoid => {
                // The vertical record model carries no radius for CLOTHOID;
                // the consumer declares the curvature ends: start_radius =
                // the incoming run's curvature basis (0 = straight run) and
                // end_curvature = the outgoing end value (signed 1/R).
                vitals.kappa_start = if segment.start_radius == 0.0 {
                    0.0
                } else {
                    1.0 / segment.start_radius
                };
                if segment.end_curvature == 0.0 {
                    return Err(
                        "vertical clothoid needs declared curvature ends (start_radius, end_curvature); the record model carries none yet"
                            .to_owned(),
                    );
                }
                vitals.kappa_end = segment.end_curvature;
                vitals.theta_start = segment.start_gradient.atan();
            }
            VerticalSegmentSpecies::Declared(ramp) => {
                // The vertical record model carries no radius for blended
                // species: k0 = 1/start_radius (0 = a straight run) and k1
                // = end_curvature, the vertical clothoid's rule verbatim
                // (k1 = 0 restates PARABOLICARC — declare that instead).
                validate_declared_ramp(&ramp).map_err(|reason| {
                    format!("vertical declared blend: {reason}")
                })?;
                if segment.end_curvature == 0.0 {
                    return Err(
                        "vertical declared blend needs declared curvature ends (start_radius, end_curvature); k1 = 0 restates PARABOLICARC"
                            .to_owned(),
                    );
                }
                vitals.kappa_start = if segment.start_radius == 0.0 {
                    0.0
                } else {
                    1.0 / segment.start_radius
                };
                vitals.kappa_end = segment.end_curvature;
                vitals.theta_start = segment.start_gradient.atan();
            }
        },
    }
    Ok(vitals)
}

/// The blend integral W(xi) of every plan blend's curvature law, normalized
/// with W(0) = 0 and W(1) = 1/2 (a one-sided straight to circle transition
/// always deflects through L/(2*R)):
///
/// - CLOTHOID: W = xi^2/2 (the curvature is linear in the arc length);
/// - BLOSSCURVE: the IFC law kappa = kappa1 + (3-2*xi)*xi^2*dkappa
///   integrates to W = xi^3 - xi^4/2;
/// - COSINECURVE: kappa = kappa1 + (1-cos(pi*xi))/2*dkappa integrates to
///   W = xi/2 - sin(pi*xi)/(2*pi);
/// - SINECURVE: kappa = kappa1 + (xi - sin(2*pi*xi)/(2*pi))*dkappa
///   integrates to W = xi^2/2 + (cos(2*pi*xi) - 1)/(4*pi^2);
/// - HELMERTCURVE: the two quadratic halves kappa = kappa1 + 2*xi^2*dkappa
///   (xi <= 1/2) respectively kappa = kappa1 + (1 - 2*(1-xi)^2)*dkappa
///   (xi > 1/2) integrate to W = 2*xi^3/3 on the first half and
///   W = 1/12 + (xi - 1/2) + (2/3)*((1-xi)^3 - 1/8) on the second;
/// - VIENNESEBEND: the quintic xi^4*(35 - 84*xi + 70*xi^2 - 20*xi^3)
///   integrates to 7*xi^5 - 14*xi^6 + 10*xi^7 - 2.5*xi^8.
///
/// The banked-bend species: the plan ramp whose gravity-center correction
/// term applies (the hard-coded VIENNESEBEND or its Declared restatement).
fn is_banked_bend(species: &HorizontalSegmentSpecies) -> bool {
    matches!(
        species,
        HorizontalSegmentSpecies::VienneseBend
            | HorizontalSegmentSpecies::Declared(DeclaredKappaRamp::Viennese)
    )
}

/// The fixed-shape gravity-center correction of the banked bend (IFC
/// 4.3.2.0 lexics), driven by the declared gravity_height/bank_change
/// pair: zero without a cant coupling, and it integrates to zero over the
/// full segment.
fn banked_bend_correction(vitals: &Vitals, xi: f64) -> f64 {
    let c = 420.0 * vitals.gravity_height * vitals.bank_change / (vitals.length * vitals.length);
    c * xi * xi * (1.0 - 4.0 * xi + 5.0 * xi * xi - 2.0 * xi * xi * xi)
}

fn blend_integral(species: HorizontalSegmentSpecies, xi: f64) -> f64 {
    match species_ramp(&species) {
        Some(ramp) => declared_ramp_integral(&ramp, xi),
        // The closed species (Line/Arc/Cubic) never reach the integrator.
        None => unreachable!("blend_integral called for a non-blend species"),
    }
}

/// The declared ramp each blend species restates (the one formula table:
/// the hard-coded species and the Declared species share it, so the two
/// cannot drift).
fn species_ramp(species: &HorizontalSegmentSpecies) -> Option<DeclaredKappaRamp> {
    match species {
        HorizontalSegmentSpecies::Clothoid => Some(DeclaredKappaRamp::Linear),
        HorizontalSegmentSpecies::BlossCurve => Some(DeclaredKappaRamp::Bloss),
        HorizontalSegmentSpecies::CosineCurve => Some(DeclaredKappaRamp::Cosine),
        HorizontalSegmentSpecies::SineCurve => Some(DeclaredKappaRamp::Sine),
        HorizontalSegmentSpecies::HelmertCurve => Some(DeclaredKappaRamp::Helmert),
        HorizontalSegmentSpecies::VienneseBend => Some(DeclaredKappaRamp::Viennese),
        HorizontalSegmentSpecies::Declared(ramp) => Some(*ramp),
        _ => None,
    }
}

/// Whether a plan species is a blend (integrated-curvature) species.
fn is_plan_blend(species: &HorizontalSegmentSpecies) -> bool {
    matches!(
        species,
        HorizontalSegmentSpecies::BlossCurve
            | HorizontalSegmentSpecies::Clothoid
            | HorizontalSegmentSpecies::CosineCurve
            | HorizontalSegmentSpecies::HelmertCurve
            | HorizontalSegmentSpecies::SineCurve
            | HorizontalSegmentSpecies::VienneseBend
            | HorizontalSegmentSpecies::Declared(_)
    )
}

/// The normalized declared ramp r(xi) in [0, 1] (ramp(0) = 0, ramp(1) = 1):
/// kappa(xi) = kappa_start + (kappa_end - kappa_start) * r(xi) — the
/// Declared head's shape, executed by the generic integrator as data.
fn declared_ramp_value(ramp: &DeclaredKappaRamp, xi: f64) -> f64 {
    match ramp {
        DeclaredKappaRamp::Linear => xi,
        DeclaredKappaRamp::Bloss => 3.0 * xi * xi - 2.0 * xi * xi * xi,
        DeclaredKappaRamp::Cosine => (1.0 - (PI * xi).cos()) * 0.5,
        DeclaredKappaRamp::Sine => xi - (TWO_PI * xi).sin() / TWO_PI,
        DeclaredKappaRamp::Helmert => {
            if xi <= 0.5 {
                2.0 * xi * xi
            } else {
                let w = 1.0 - xi;
                1.0 - 2.0 * w * w
            }
        }
        DeclaredKappaRamp::Viennese => {
            let xi2 = xi * xi;
            let xi4 = xi2 * xi2;
            xi4 * (35.0 - 84.0 * xi + 70.0 * xi2 - 20.0 * xi2 * xi)
        }
        DeclaredKappaRamp::Piecewise { count, knots } => {
            match active_knots(*count, knots) {
                Some(active) => piecewise_value(active, xi),
                // An invalid table cannot reach evaluation (validation
                // refuses it first); integrate nothing rather than guess.
                None => 0.0,
            }
        }
    }
}

/// The blend integral W(xi) of a piecewise-linear ramp through the active
/// knots: per interval the quadratic r0*h + slope*h^2/2 accumulation. The
/// active slice is validated at declaration (first = (0, 0), last =
/// (1, 1), ascending, values in [0, 1]); the table's W(1) is its own
/// integral — 1/2 only for the IFC-shaped ramps.
fn piecewise_value(knots: &[(f64, f64)], xi: f64) -> f64 {
    let mut acc = 0.0;
    for pair in knots.windows(2) {
        let (xi0, r0) = pair[0];
        let (xi1, r1) = pair[1];
        let h = (xi1 - xi0).max(f64::MIN_POSITIVE);
        let span = (xi - xi0).min(h).max(0.0);
        acc += r0 * span + (r1 - r0) * span * span / (2.0 * h);
        if span < h {
            break;
        }
    }
    acc
}

/// The declared ramp's blend integral W(xi) — the closed antiderivative
/// per entry (the same formulas the hard-coded species carry, with the
/// Piecewise case summed per interval).
fn declared_ramp_integral(ramp: &DeclaredKappaRamp, xi: f64) -> f64 {
    match ramp {
        DeclaredKappaRamp::Linear => xi * xi * 0.5,
        DeclaredKappaRamp::Bloss => {
            let xi2 = xi * xi;
            xi2 * xi - xi2 * xi2 * 0.5
        }
        DeclaredKappaRamp::Cosine => xi * 0.5 - (PI * xi).sin() / TWO_PI,
        DeclaredKappaRamp::Sine => {
            xi * xi * 0.5 + ((TWO_PI * xi).cos() - 1.0) / (4.0 * PI * PI)
        }
        DeclaredKappaRamp::Helmert => {
            if xi <= 0.5 {
                2.0 * xi * xi * xi / 3.0
            } else {
                let w = 1.0 - xi;
                1.0 / 12.0 + (xi - 0.5) + 2.0 * (w * w * w - 0.125) / 3.0
            }
        }
        DeclaredKappaRamp::Viennese => {
            let xi2 = xi * xi;
            let xi4 = xi2 * xi2;
            7.0 * xi4 * xi - 14.0 * xi4 * xi2 + 10.0 * xi4 * xi2 * xi - 2.5 * xi4 * xi2 * xi2
        }
        DeclaredKappaRamp::Piecewise { count, knots } => {
            match active_knots(*count, knots) {
                Some(active) => piecewise_value(active, xi),
                // An invalid knot table cannot reach evaluation (validation
                // refuses it first); integrate nothing rather than guess.
                None => 0.0,
            }
        }
    }
}

/// The Declared law's own validation: every unit ramp is total; the
/// Piecewise table must carry a valid active knot slice — the first knot
/// (0, 0), the last active knot (1, 1), strictly ascending finite xi with
/// every active ramp value finite in [0, 1]. The head stays bounded like
/// the grip caps: the audit surface is the declared data, nothing else.
fn validate_declared_ramp(ramp: &DeclaredKappaRamp) -> Result<(), String> {
    let DeclaredKappaRamp::Piecewise { count, knots } = ramp else {
        return Ok(());
    };
    let count = *count as usize;
    if count == 0 || count > knots.len() {
        return Err(format!(
            "the piecewise knot count {count} is not a valid knot count (1..={})",
            knots.len()
        ));
    }
    let active = &knots[..count];
    if active[0] != (0.0, 0.0) {
        return Err(format!(
            "the piecewise ramp must start at the knot (0, 0); got {:?}",
            active[0]
        ));
    }
    let last = active[active.len() - 1];
    if last.0 != 1.0 || last.1 != 1.0 {
        return Err(format!(
            "the piecewise ramp must end at the knot (1, 1); got {last:?}"
        ));
    }
    for pair in active.windows(2) {
        let ((xi0, r0), (xi1, r1)) = (pair[0], pair[1]);
        if !(xi0.is_finite() && xi1.is_finite() && r0.is_finite() && r1.is_finite()) {
            return Err("the piecewise knots must be finite".to_owned());
        }
        if xi1 <= xi0 {
            return Err(format!(
                "the piecewise knots must ascend strictly; the knot xi {xi0} repeats"
            ));
        }
        if !(
            (0.0..=1.0).contains(&r0)
                && (0.0..=1.0).contains(&r1)
        ) {
            return Err(format!(
                "the piecewise ramp values must stay in [0, 1]; got ({xi0}, {r0}) .. ({xi1}, {r1})"
            ));
        }
    }
    Ok(())
}

/// The slice of the piecewise knot table that is active law; `None` when
/// the count is not a valid knot count.
fn active_knots(count: u32, knots: &[(f64, f64); 6]) -> Option<&[(f64, f64)]> {
    let count = count as usize;
    if count == 0 || count > knots.len() {
        return None;
    }
    Some(&knots[..count])
}

/// The plan tangent angle at arc position s, relative to the segment's
/// local frame (+x along the declared start direction).
// ==== the v2 generic ramp machinery (additive now; the unfrozen switch wave
// ==== deletes the named-species arms above and rewires everything onto this) ====
//
// The host's generic curve contract: the driving quantity arrives as DATA
// (`Ramp`) over the unit station parameter; the host evaluates it, integrates
// it (the audit + the carriers need the integral), and precomputes stage-time
// cumulative tables so the per-move re-derivation tier stays O(log n) + a
// small exact tail. No species words anywhere in this block.

/// Evaluate a generic `Ramp` at `xi` (clamped to [0, 1]).
/// Polynomial: ascending Horner over `coeffs[0..=degree]`. Piecewise:
/// the same linear-interpolation law `piecewise_value` uses. Sampled:
/// uniform-grid linear lookup.
pub(crate) fn generic_ramp_eval(
    ramp: &ocs_plugin_api::host::Ramp,
    xi: f64,
) -> f64 {
    let xi = xi.clamp(0.0, 1.0);
    match ramp {
        ocs_plugin_api::host::Ramp::Polynomial { degree, coeffs } => {
            let d = (*degree).min(3) as usize;
            let mut acc = 0.0;
            for i in (0..=d).rev() {
                acc = acc * xi + coeffs[i];
            }
            acc
        }
        ocs_plugin_api::host::Ramp::Piecewise { count, knots } => {
            let active = (*count).clamp(1, knots.len() as u32) as usize;
            let knots = &knots[..active];
            // The linear-interpolation law: between knots the value runs
            // from r0 to r1; before the first / after the last, clamp.
            if xi <= knots[0].0 {
                return knots[0].1;
            }
            for pair in knots.windows(2) {
                let (x0, v0) = pair[0];
                let (x1, v1) = pair[1];
                if x1 <= x0 {
                    continue;
                }
                if xi <= x1 {
                    let frac = (xi - x0) / (x1 - x0);
                    return v0 + (v1 - v0) * frac;
                }
            }
            knots[knots.len() - 1].1
        }
        ocs_plugin_api::host::Ramp::Sampled { n, values } => {
            let count = (*n).clamp(1, values.len() as u32) as usize;
            if count == 1 {
                return values[0];
            }
            let t = xi * (count - 1) as f64;
            let i0 = (t.floor() as usize).min(count - 2).max(0);
            let frac = t - i0 as f64;
            values[i0] + (values[i0 + 1] - values[i0]) * frac
        }
    }
}

/// Integrate a generic `Ramp` over [0, xi]. Polynomial: the antiderivative
/// in closed form. Piecewise: the audited per-interval quadratic
/// accumulation (`piecewise_value`'s own law — it integrates the linear
/// ramp; the name predates the v2 contract, the switch wave keeps it as
/// the piecewise antiderivative). Sampled: trapezoidal accumulation (the
/// grid's own law — the audit's deviation floor derives from the table
/// spacing, engine-verified).
pub(crate) fn generic_ramp_integral(
    ramp: &ocs_plugin_api::host::Ramp,
    xi: f64,
) -> f64 {
    let xi = xi.clamp(0.0, 1.0);
    match ramp {
        ocs_plugin_api::host::Ramp::Polynomial { degree, coeffs } => {
            let d = (*degree).min(3) as usize;
            let mut acc = 0.0;
            for i in (0..=d).rev() {
                acc = (acc + coeffs[i] / (i as f64 + 1.0)) * xi;
            }
            // the loop yields Σ coeffs[i]·xi^(i+1)/(i+1) in ascending order
            acc
        }
        ocs_plugin_api::host::Ramp::Piecewise { count, knots } => {
            let active = (*count).clamp(1, knots.len() as u32) as usize;
            piecewise_value(&knots[..active], xi)
        }
        ocs_plugin_api::host::Ramp::Sampled { n, values } => {
            let count = (*n).clamp(1, values.len() as u32) as usize;
            if count == 1 {
                return 0.0;
            }
            let step = 1.0 / (count - 1) as f64;
            let full = (xi / step).floor() as usize;
            let full = full.min(count - 1);
            let mut total = 0.0;
            for i in 0..full {
                total += (values[i] + values[i + 1]) * 0.5;
            }
            total *= step;
            let done_at = full as f64 * step;
            let tail = xi - done_at;
            if tail > 0.0 && full < count - 1 {
                total += (values[full] + values[full + 1]) * 0.5 * tail;
            }
            total
        }
    }
}

fn plan_local_theta(species: &HorizontalSegmentSpecies, vitals: &Vitals, s: f64) -> f64 {
    match species {
        HorizontalSegmentSpecies::Line => 0.0,
        HorizontalSegmentSpecies::CircularArc => vitals.kappa_start * s,
        HorizontalSegmentSpecies::Cubic => {
            // Cubic parabola y = x^3/(6*R*L): slope g(x) = x^2/(2*R*L).
            if vitals.length == 0.0 {
                0.0
            } else {
                let slope = s * s / (2.0 * (1.0 / vitals.kappa_end) * vitals.length);
                slope.atan()
            }
        }
        _ => {
            let xi = if vitals.length > 0.0 { s / vitals.length } else { 0.0 };
            let dk = vitals.kappa_end - vitals.kappa_start;
            let mut theta =
                vitals.kappa_start * s + dk * vitals.length * blend_integral(*species, xi);
            if is_banked_bend(species) {
                // Gravity-center correction term of the banked bend (IFC
                // 4.3.2.0 lexics); zero without a cant coupling, and the
                // correction integrates to zero over the full segment.
                theta -= banked_bend_correction(vitals, xi);
            }
            theta
        }
    }
}

/// 16-node Gauss-Legendre nodes (on [-1, 1]) and weights, hard-coded
/// constants so evaluation is bit-deterministic. The nodes are quoted
/// verbatim from the reference table (digit-for-digit), hence the allow.
#[allow(clippy::excessive_precision)]
const GL16_Z: [f64; 16] = [
    -0.98940093499164995,
    -0.94457502307323258,
    -0.86563120238783174,
    -0.75540440835500303,
    -0.61787624440264375,
    -0.45801677765722739,
    -0.28160355077925891,
    -0.09501250983763744,
    0.09501250983763744,
    0.28160355077925891,
    0.45801677765722739,
    0.61787624440264375,
    0.75540440835500303,
    0.86563120238783174,
    0.94457502307323258,
    0.98940093499164995,
];
const GL16_W: [f64; 16] = [
    0.0271524594117541,
    0.0622535239386479,
    0.0951585116824928,
    0.1246289712555339,
    0.1495959888165767,
    0.1691565193950025,
    0.1826034150449236,
    0.1894506104550685,
    0.1894506104550685,
    0.1826034150449236,
    0.1691565193950025,
    0.1495959888165767,
    0.1246289712555339,
    0.0951585116824928,
    0.0622535239386479,
    0.0271524594117541,
];

/// Composite 16-node Gauss-Legendre over [0, domain], subdivided by the
/// integrand's total phase swing (one sub-interval per half-radian of
/// deflection). Deterministic and allocation-light.
fn integrate(mut f: impl FnMut(f64) -> f64, domain: f64, total_swing: f64) -> f64 {
    if domain == 0.0 {
        return 0.0;
    }
    let mut sub_intervals = 1.0f64.max(total_swing.abs() / (PI * 0.5)).ceil() as usize;
    sub_intervals = sub_intervals.max(1);
    let h = domain / sub_intervals as f64;
    let mut sum = 0.0;
    for i in 0..sub_intervals {
        let a = i as f64 * h;
        for (z, w) in GL16_Z.iter().zip(GL16_W.iter()) {
            let t = a + 0.5 * h * (z + 1.0);
            sum += w * f(t);
        }
    }
    sum * 0.5 * h
}

/// Plan blend species position at arc position u: the local-frame (x, y)
/// integrals of (cos theta(t), sin theta(t)).
fn integrate_plan_curve(
    species: &HorizontalSegmentSpecies,
    vitals: &Vitals,
    u: f64,
) -> (f64, f64) {
    let swing = plan_local_theta(species, vitals, u) - plan_local_theta(species, vitals, 0.0);
    let theta = |t: f64| plan_local_theta(species, vitals, t);
    let x = integrate(|t| theta(t).cos(), u, swing);
    let y = integrate(|t| theta(t).sin(), u, swing);
    (x, y)
}

/// Evaluate one plan segment at arc position u, in plane (world)
/// coordinates.
fn evaluate_plan(
    species: &HorizontalSegmentSpecies,
    segment: &ParametricSegment,
    vitals: &Vitals,
    u: f64,
) -> ParametricSample {
    let (lx, ly) = match species {
        HorizontalSegmentSpecies::Line => (u, 0.0),
        HorizontalSegmentSpecies::CircularArc => {
            let r = vitals.radius;
            (r * (u / r).sin(), r * (1.0 - (u / r).cos()))
        }
        HorizontalSegmentSpecies::Cubic => {
            // Cubic parabola in the run parameter.
            let r = 1.0 / vitals.kappa_end;
            (u, u * u * u / (6.0 * r * vitals.length))
        }
        _ => integrate_plan_curve(species, vitals, u),
    };
    let dir_angle = vitals.theta_start
        + match species {
            HorizontalSegmentSpecies::Line => 0.0,
            HorizontalSegmentSpecies::CircularArc => u / vitals.radius,
            HorizontalSegmentSpecies::Cubic => {
                let r = 1.0 / vitals.kappa_end;
                (u * u / (2.0 * r * vitals.length)).atan()
            }
            _ => plan_local_theta(species, vitals, u),
        };
    let cos0 = vitals.theta_start.cos();
    let sin0 = vitals.theta_start.sin();
    ParametricSample {
        point: [
            segment.start_point[0] + cos0 * lx - sin0 * ly,
            segment.start_point[1] + sin0 * lx + cos0 * ly,
        ],
        direction: [dir_angle.cos(), dir_angle.sin()],
    }
}

/// The vertical clothoid's slope angle on the run:
/// theta(u) = theta0 + kappa1*u + dkappa*u^2/(2*L) (the linear-curvature
/// law, IFC 4.3.2.0 lexics).
fn profile_clothoid_theta(vitals: &Vitals, u: f64) -> f64 {
    let dk = vitals.kappa_end - vitals.kappa_start;
    vitals.theta_start + vitals.kappa_start * u
        + dk * u * u / (2.0 * vitals.length.max(f64::MIN_POSITIVE))
}

/// Evaluate the vertical clothoid at run position u, in the (station,
/// elevation) sheet.
fn evaluate_profile_clothoid(
    segment: &ParametricSegment,
    vitals: &Vitals,
    u: f64,
) -> ParametricSample {
    evaluate_profile_blend(segment, vitals, u, None)
}

/// Generalized profile blend: the declared ramp's run law
/// theta(u) = theta0 + k0*u + (k1-k0)*L*W(s/L) — the vertical clothoid's
/// shape with W from the declared ramp (W = xi^2/2 = the clothoid's own);
/// the height integrates the realized slope via the generic integrator.
fn evaluate_profile_blend(
    segment: &ParametricSegment,
    vitals: &Vitals,
    u: f64,
    ramp: Option<DeclaredKappaRamp>,
) -> ParametricSample {
    // The realized heading law: the vertical clothoid's shape for the
    // built-in ramp (None), the declared ramp otherwise — the same
    // theta0 + k0*u + (k1-k0)*L*W(u/L) run law either way.
    let theta_of = |t: f64| match ramp {
        None => profile_clothoid_theta(vitals, t),
        Some(r) => {
            let xi = if vitals.length > 0.0 { t / vitals.length } else { 0.0 };
            vitals.theta_start
                + vitals.kappa_start * t
                + (vitals.kappa_end - vitals.kappa_start)
                    * vitals.length
                    * declared_ramp_integral(&r, xi)
        }
    };
    let theta_u = theta_of(u);
    let swing = theta_u - segment.start_gradient.atan();
    // The height integrates the REALIZED slope — the declared law's own
    // heading; a ramp-blind integrand would restate the clothoid's height.
    let height = segment.start_point[1] + integrate(|t| theta_of(t).tan(), u, swing);
    let gradient = theta_u.tan();
    let norm = (1.0 + gradient * gradient).sqrt();
    ParametricSample {
        point: [segment.start_point[0] + u, height],
        direction: [1.0 / norm, gradient / norm],
    }
}

/// Evaluate one profile segment at run position u, in the (station,
/// elevation) sheet (x = distance along the horizontal, y = height — the
/// IfcAlignmentVerticalSegment frame, no rotation applies).
fn evaluate_profile(
    species: &VerticalSegmentSpecies,
    segment: &ParametricSegment,
    vitals: &Vitals,
    u: f64,
) -> ParametricSample {
    let g0 = segment.start_gradient;
    let station0 = segment.start_point[0];
    let h0 = segment.start_point[1];
    let theta0 = vitals.theta_start;
    let (run, height, gradient) = match species {
        VerticalSegmentSpecies::ConstantGradient => (u, h0 + g0 * u, g0),
        VerticalSegmentSpecies::ParabolicArc => {
            let r = vitals.radius;
            (u, h0 + g0 * u + u * u / (2.0 * r), g0 + u / r)
        }
        VerticalSegmentSpecies::CircularArc => {
            // The exact circle in the sheet:
            // sin(theta(u)) = sin(theta0) + u/R,
            // h(u) = h0 + R*(cos(theta0) - cos(theta(u))).
            let sine = theta0.sin() + u / vitals.radius;
            let theta = sine.asin();
            (
                u,
                h0 + vitals.radius * (theta0.cos() - theta.cos()),
                theta.tan(),
            )
        }
        VerticalSegmentSpecies::Clothoid => {
            let sample = evaluate_profile_clothoid(segment, vitals, u);
            (
                u,
                sample.point[1],
                sample.direction[1] / sample.direction[0],
            )
        }
        VerticalSegmentSpecies::Declared(ramp) => {
            let sample = evaluate_profile_blend(segment, vitals, u, Some(*ramp));
            (
                u,
                sample.point[1],
                sample.direction[1] / sample.direction[0],
            )
        }
    };
    let norm = (1.0 + gradient * gradient).sqrt();
    ParametricSample {
        point: [station0 + run, height],
        direction: [1.0 / norm, gradient / norm],
    }
}

/// Evaluate one (already validated) segment at local position u.
fn evaluate_segment_at(
    _plane: ParametricPlane,
    segment: &ParametricSegment,
    vitals: &Vitals,
    u: f64,
) -> ParametricSample {
    match segment.species {
        ParametricSpecies::Horizontal(species) => evaluate_plan(&species, segment, vitals, u),
        ParametricSpecies::Vertical(species) => evaluate_profile(&species, segment, vitals, u),
    }
}

/// Audit every declared segment pair seam-to-seam: the evaluated end of
/// segment i must equal the declared start of segment i+1 in point, and in
/// direction (the record model's continuity tolerances). Refuses with the
/// audited pair named, nothing staged. Returns the validated vitals so the
/// staging pass never re-validates.
fn audit_chain(chain: &ParametricChain) -> Result<Vec<Vitals>, String> {
    let mut vitals = Vec::with_capacity(chain.segments.len());
    for (index, segment) in chain.segments.iter().enumerate() {
        let v =
            validate_segment(chain.plane, segment).map_err(|e| format!("segment {index}: {e}"))?;
        vitals.push(v);
    }
    for (index, (pair, v)) in chain.segments.windows(2).zip(vitals.iter()).enumerate() {
        let segment = &pair[0];
        let next = &pair[1];
        let end = evaluate_segment_at(chain.plane, segment, v, segment.length);
        if !next.start_point.iter().all(|c| c.is_finite()) {
            return Err(format!(
                "seam segment {index} to {}: the declared start point is not finite",
                index + 1
            ));
        }
        let gap = ((end.point[0] - next.start_point[0]).powi(2)
            + (end.point[1] - next.start_point[1]).powi(2))
        .sqrt();
        if gap > SEAM_POINT_EPS {
            return Err(format!(
                "seam segment {index} to {}: the evaluated end ({:.6}, {:.6}) is {gap:.9} m away from the next segment's declared start ({:.6}, {:.6}); the chain is not continuous",
                index + 1,
                end.point[0],
                end.point[1],
                next.start_point[0],
                next.start_point[1]
            ));
        }
        match chain.plane {
            ParametricPlane::Plan => {
                let realized = end.direction[1].atan2(end.direction[0]);
                let turn = angle_distance(realized, next.start_direction);
                if turn > SEAM_DIRECTION_EPS {
                    return Err(format!(
                        "seam segment {index} to {}: the evaluated end direction ({realized:.12} rad) does not restate the declared start direction ({:.12} rad) of the next segment; the chain is not tangentially continuous",
                        index + 1,
                        next.start_direction
                    ));
                }
            }
            ParametricPlane::Profile => {
                // The realized end slope vs the next segment's declared
                // start gradient — the record model's tangential-continuity
                // check in the gradient form the records carry.
                let realized_gradient = end.direction[1] / end.direction[0];
                let declared = next.start_gradient;
                if (realized_gradient - declared).abs()
                    > SEAM_GRADIENT_EPS * (1.0 + declared.abs() * declared.abs())
                {
                    return Err(format!(
                        "seam segment {index} to {}: the evaluated end gradient ({realized_gradient:.9}) does not restate the declared start gradient ({declared:.9}) of the next segment",
                        index + 1
                    ));
                }
            }
        }
    }
    Ok(vitals)
}

/// Sample a declared chain at chain position s (measured along the chain's
/// declared segment lengths). Pure evaluation: no document state, no undo,
/// no publish. Outside the chain (s < 0 or past the end) the answer is
/// `None`.
pub fn sample_chain(chain: &ParametricChain, s: f64) -> Result<Option<ParametricSample>, String> {
    if !s.is_finite() {
        return Err("sample position is not finite".to_owned());
    }
    let mut vitals = Vec::with_capacity(chain.segments.len());
    let mut total = 0.0;
    for (index, segment) in chain.segments.iter().enumerate() {
        let v =
            validate_segment(chain.plane, segment).map_err(|e| format!("segment {index}: {e}"))?;
        total += v.length;
        vitals.push(v);
    }
    if s < 0.0 || total == 0.0 || s > total {
        return Ok(None);
    }
    let mut acc = 0.0;
    for (index, segment) in chain.segments.iter().enumerate() {
        let span = vitals[index].length;
        if span == 0.0 {
            continue;
        }
        if s <= acc + span || index + 1 == chain.segments.len() {
            let u = (s - acc).clamp(0.0, span);
            return Ok(Some(evaluate_segment_at(
                chain.plane,
                segment,
                &vitals[index],
                u,
            )));
        }
        acc += span;
    }
    Ok(None)
}

/// Chord count seed from the curvature-bound sagitta law: a chord of arc
/// span ds on curvature kappa bulges at most ds^2*kappa/8, so
/// n >= L*sqrt(kappa/(8*tolerance)). The measured deviation check is the
/// real law; this bound only seeds it.
fn chord_seed(species: &HorizontalSegmentSpecies, vitals: &Vitals, tolerance: f64) -> usize {
    let mut bound = vitals.kappa_start.abs() + (vitals.kappa_end - vitals.kappa_start).abs();
    if is_banked_bend(species) && vitals.length > 0.0 {
        // The banked correction can overshoot the monotone bound: sample
        // the realized law (the declared ramp or the Viennese shape).
        let band = vitals.kappa_end - vitals.kappa_start;
        let ramp = species_ramp(species);
        let mut sampled = bound;
        for k in 0..=48usize {
            let xi = k as f64 / 48.0;
            let mut kappa = match ramp {
                Some(ramp) => {
                    vitals.kappa_start + band * declared_ramp_value(&ramp, xi)
                }
                None => vitals.kappa_start,
            };
            kappa -= banked_bend_correction(vitals, xi);
            sampled = sampled.max(kappa.abs());
        }
        bound = sampled;
    }
    if bound <= 0.0 {
        return 2;
    }
    let n = (vitals.length * (bound / (8.0 * tolerance)).sqrt()).ceil() as usize;
    n.max(2)
}

/// Chord polyline with the max-deviation tolerance contract: seeded from
/// the sagitta bound, refined by measured deviation until the measured
/// maximum sits within the declared tolerance. Refuses with the reached
/// numbers when the refinement ceiling keeps the fit above the tolerance.
fn chord_polyline(
    eval: &dyn Fn(f64) -> ParametricSample,
    length: f64,
    mut count: usize,
    tolerance: f64,
    label: &str,
) -> Result<(LwPolyline, usize), String> {
    loop {
        let polyline = chords_for(eval, length, count);
        let worst = measured_deviation(&polyline, eval, length);
        if worst <= tolerance {
            return Ok((polyline, count));
        }
        if count >= MAX_CHORDS_PER_SEGMENT {
            return Err(format!(
                "{label}: {count} chords still deviate by {worst:.3e} m at a {tolerance} tolerance; the tolerance is unattainable, nothing staged for it"
            ));
        }
        count = count.saturating_mul(2).max(4);
    }
}

fn chords_for(eval: &dyn Fn(f64) -> ParametricSample, length: f64, count: usize) -> LwPolyline {
    let mut polyline = LwPolyline::new();
    for i in 0..=count {
        let u = length * i as f64 / count as f64;
        let sample = eval(u);
        polyline.add_vertex(LwVertex::new(Vector2::new(sample.point[0], sample.point[1])));
    }
    polyline
}

/// The measured max deviation of the chord polyline vs the true curve:
/// three quarter-points sampled per chord, the distance to the chord line.
fn measured_deviation(
    polyline: &LwPolyline,
    eval: &dyn Fn(f64) -> ParametricSample,
    length: f64,
) -> f64 {
    let count = polyline.vertices.len().saturating_sub(1).max(1);
    let vertices: Vec<(f64, f64)> = polyline
        .vertices
        .iter()
        .map(|v| (v.location.x, v.location.y))
        .collect();
    let mut worst = 0.0f64;
    for i in 0..count {
        let (ax, ay) = vertices[i];
        let (bx, by) = vertices[i + 1];
        let (dx, dy) = (bx - ax, by - ay);
        let chord_len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = if chord_len == 0.0 { (1.0, 0.0) } else { (dx / chord_len, dy / chord_len) };
        for k in 1..4 {
            let frac = k as f64 / 4.0;
            let u = length * (i as f64 + frac) / count as f64;
            let sample = eval(u);
            let px = sample.point[0] - ax;
            let py = sample.point[1] - ay;
            let along = px * ux + py * uy;
            let dist = ((px - along * ux).powi(2) + (py - along * uy).powi(2)).sqrt();
            worst = worst.max(dist);
        }
    }
    worst
}

/// Which carrier answered for one staged segment (diagnostics).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StagedCarrier {
    /// One exact standard entity.
    Exact,
    /// A chord polyline with this many chords.
    Chords(usize),
}

/// Build the display entities for a declared, audited chain. Every staged
/// entity carries the chain's layer (when set); the reference-only
/// designation is applied by the host session, not here. Returns the
/// entities and the per-segment carriers.
pub fn build_display_entities(
    chain: &ParametricChain,
    tolerance: f64,
) -> Result<(Vec<EntityType>, Vec<StagedCarrier>), String> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err("a positive finite tolerance is required".to_owned());
    }
    let vitals = audit_chain(chain)?;
    // Cap pre-check (the published ceiling): nothing is staged when the
    // chain's display footprint would exceed the live-entity cap.
    let mut estimate = 0usize;
    for (index, segment) in chain.segments.iter().enumerate() {
        let v = vitals[index];
        if v.length == 0.0 {
            continue;
        }
        estimate += match &segment.species {
            ParametricSpecies::Horizontal(species) => {
                if is_plan_blend(species) {
                    chord_seed(species, &v, tolerance)
                } else {
                    1
                }
            }
            ParametricSpecies::Vertical(species) => {
                if matches!(
                    species,
                    VerticalSegmentSpecies::Clothoid | VerticalSegmentSpecies::Declared(_)
                ) {
                    chord_seed(&HorizontalSegmentSpecies::Clothoid, &v, tolerance)
                } else {
                    1
                }
            }
        };
    }
    if estimate > MAX_PLUGIN_LIVE_ENTITIES {
        return Err(format!(
            "a {}-segment chain at tolerance {tolerance} needs {estimate} display entities, above the MAX_PLUGIN_LIVE_ENTITIES ({MAX_PLUGIN_LIVE_ENTITIES}) ceiling; nothing staged",
            chain.segments.len()
        ));
    }
    let mut entities: Vec<EntityType> = Vec::new();
    let mut carriers = Vec::with_capacity(chain.segments.len());
    for (index, segment) in chain.segments.iter().enumerate() {
        let v = vitals[index];
        if v.length == 0.0 {
            // A terminator segment stages nothing (the record layer's
            // zero-length terminators are legal but carry no geometry).
            carriers.push(StagedCarrier::Exact);
            continue;
        }
        match &segment.species {
            ParametricSpecies::Horizontal(species) => match species {
                HorizontalSegmentSpecies::Line => {
                    entities.push(EntityType::Line(plan_line(segment, &v)));
                    carriers.push(StagedCarrier::Exact);
                }
                HorizontalSegmentSpecies::CircularArc => {
                    entities.push(EntityType::Arc(plan_arc(segment, &v)));
                    carriers.push(StagedCarrier::Exact);
                }
                HorizontalSegmentSpecies::Cubic => {
                    entities.push(EntityType::Spline(cubic_spline(segment, &v)));
                    carriers.push(StagedCarrier::Exact);
                }
                _ => {
                    let eval = |u: f64| evaluate_plan(species, segment, &v, u);
                    let seed = chord_seed(species, &v, tolerance);
                    let (polyline, count) = chord_polyline(
                        &eval,
                        v.length,
                        seed,
                        tolerance,
                        &format!("segment {index} ({species:?})"),
                    )?;
                    entities.push(EntityType::LwPolyline(polyline));
                    carriers.push(StagedCarrier::Chords(count));
                }
            },
            ParametricSpecies::Vertical(species) => match species {
                VerticalSegmentSpecies::ConstantGradient => {
                    entities.push(EntityType::Line(profile_line(segment, &v)));
                    carriers.push(StagedCarrier::Exact);
                }
                VerticalSegmentSpecies::ParabolicArc => {
                    entities.push(EntityType::Spline(parabola_spline(segment, &v)));
                    carriers.push(StagedCarrier::Exact);
                }
                VerticalSegmentSpecies::CircularArc => {
                    entities.push(EntityType::Arc(profile_arc(segment, &v)));
                    carriers.push(StagedCarrier::Exact);
                }
                VerticalSegmentSpecies::Clothoid => {
                    let eval = |u: f64| evaluate_profile_clothoid(segment, &v, u);
                    let seed = chord_seed(&HorizontalSegmentSpecies::Clothoid, &v, tolerance);
                    let (polyline, count) = chord_polyline(
                        &eval,
                        v.length,
                        seed,
                        tolerance,
                        &format!("segment {index} (vertical clothoid)"),
                    )?;
                    entities.push(EntityType::LwPolyline(polyline));
                    carriers.push(StagedCarrier::Chords(count));
                }
                VerticalSegmentSpecies::Declared(ramp) => {
                    let eval =
                        |u: f64| evaluate_profile_blend(segment, &v, u, Some(*ramp));
                    // The bounded declared kappa (ramp in [0, 1]) seeds the
                    // chord plan with the same sagitta bound.
                    let seed = chord_seed(&HorizontalSegmentSpecies::Clothoid, &v, tolerance);
                    let (polyline, count) = chord_polyline(
                        &eval,
                        v.length,
                        seed,
                        tolerance,
                        &format!("segment {index} (vertical declared blend)"),
                    )?;
                    entities.push(EntityType::LwPolyline(polyline));
                    carriers.push(StagedCarrier::Chords(count));
                }
            },
        }
    }
    if !chain.layer.is_empty() {
        for entity in &mut entities {
            set_common_layer(entity, &chain.layer);
        }
    }
    Ok((entities, carriers))
}

/// Apply the chain layer to one entity.
fn set_common_layer(entity: &mut EntityType, layer: &str) {
    if !layer.is_empty() {
        entity.common_mut().layer = layer.to_string();
    }
}

/// The plan LINE carrier: endpoints the exact evaluations.
fn plan_line(segment: &ParametricSegment, vitals: &Vitals) -> Line {
    let dir = [vitals.theta_start.cos(), vitals.theta_start.sin()];
    Line::from_points(
        Vector3::new(segment.start_point[0], segment.start_point[1], 0.0),
        Vector3::new(
            segment.start_point[0] + dir[0] * vitals.length,
            segment.start_point[1] + dir[1] * vitals.length,
            0.0,
        ),
    )
}

/// The profile CONSTANTGRADIENT carrier.
fn profile_line(segment: &ParametricSegment, vitals: &Vitals) -> Line {
    Line::from_points(
        Vector3::new(segment.start_point[0], segment.start_point[1], 0.0),
        Vector3::new(
            segment.start_point[0] + vitals.length,
            segment.start_point[1] + segment.start_gradient * vitals.length,
            0.0,
        ),
    )
}

/// The plan CIRCULARARC carrier: an exact arc entity. The record's signed
/// radius convention maps to the entity's positive radius with the
/// start/end angles chosen so the drawn point set is identical: positive
/// (CCW) radii sweep from theta0 - pi/2 forward by L/|R|; negative (CW)
/// radii draw the same point set with the span angles recomputed from the
/// CW side.
fn plan_arc(segment: &ParametricSegment, vitals: &Vitals) -> Arc {
    let r = vitals.radius;
    let theta0 = vitals.theta_start;
    // The center sits |R| perpendicular to the tangent, the turn side the
    // signed radius encodes: center = P0 + R*(-sin, cos).
    let center = [
        segment.start_point[0] - r * theta0.sin(),
        segment.start_point[1] + r * theta0.cos(),
    ];
    let (start_angle, end_angle) = if r > 0.0 {
        let a0 = theta0 - 0.5 * PI;
        let sweep = vitals.length / r;
        (a0, a0 + sweep)
    } else {
        let a1 = theta0 + 0.5 * PI;
        let sweep = vitals.length / v_abs(r);
        (a1 - sweep, a1)
    };
    Arc::from_center_radius_angles(
        Vector3::new(center[0], center[1], 0.0),
        v_abs(r),
        start_angle,
        end_angle,
    )
}

fn v_abs(v: f64) -> f64 {
    v.abs()
}

/// The profile CIRCULARARC carrier: the exact planar circle of the
/// (run, elevation) sheet; radius |R|, angles per the same signed-radius
/// mapping as the plan arc. The drawn angular span is the run law's own:
/// sin(theta(u)) = sin(theta0) + u/R puts the end tangent at
/// theta1 = asin(sin(theta0) + L/R), so the entity spans
/// [theta0 - pi/2, theta1 - pi/2] (positive radii, CCW) and the same point
/// set recomputed from the closing side (negative radii, CW) — NOT
/// run/|R|, which differs from the true span for every real gradient pair.
fn profile_arc(segment: &ParametricSegment, vitals: &Vitals) -> Arc {
    let r = vitals.radius;
    let theta0 = vitals.theta_start;
    let center = [
        segment.start_point[0] - theta0.sin() * r,
        segment.start_point[1] + theta0.cos() * r,
    ];
    let sine1 = theta0.sin() + vitals.length / r;
    // The validated segment keeps |sine1| < 1; the clamp only guards the
    // asin against a NaN-fence drift.
    let theta1 = sine1.clamp(-1.0, 1.0).asin();
    let (start_angle, end_angle) = if r > 0.0 {
        (theta0 - 0.5 * PI, theta1 - 0.5 * PI)
    } else {
        (theta1 + 0.5 * PI, theta0 + 0.5 * PI)
    };
    Arc::from_center_radius_angles(
        Vector3::new(center[0], center[1], 0.0),
        v_abs(r),
        normalize_angle(start_angle),
        normalize_angle(end_angle),
    )
}

/// The plan CUBIC carrier: the exact single-span clamped degree-3 spline of
/// the cubic parabola y = x^3/(6*R*L). The Bernstein form is exact:
/// x(t) = L*t through control x = [0, L/3, 2L/3, L] (the degree-elevated
/// linear), y(t) = (L^2/(6R))*t^3 through y = [0, 0, 0, L^2/(6R)].
fn cubic_spline(segment: &ParametricSegment, vitals: &Vitals) -> Spline {
    let theta0 = vitals.theta_start;
    let cos0 = theta0.cos();
    let sin0 = theta0.sin();
    let l = vitals.length;
    let r = 1.0 / vitals.kappa_end;
    let y_end = l * l / (6.0 * r);
    let local = [(0.0, 0.0), (l / 3.0, 0.0), (2.0 * l / 3.0, 0.0), (l, y_end)];
    let points: Vec<Vector3> = local
        .iter()
        .map(|p| {
            Vector3::new(
                segment.start_point[0] + cos0 * p.0 - sin0 * p.1,
                segment.start_point[1] + sin0 * p.0 + cos0 * p.1,
                0.0,
            )
        })
        .collect();
    let theta_end = theta0 + (l / (2.0 * r)).atan();
    let mut spline = Spline::from_control_points(3, points);
    spline.flags.planar = true;
    spline.begin_tangent = Vector3::new(cos0, sin0, 0.0);
    spline.end_tangent = Vector3::new(theta_end.cos(), theta_end.sin(), 0.0);
    spline
}

/// The profile PARABOLICARC carrier: the exact single-span clamped degree-2
/// spline of h(u) = h0 + g0*u + u^2/(2*R) in the (station, elevation)
/// sheet. Control points: x = [0, L/2, L] (the degree-elevated linear),
/// y = [h0, h0 + g0*L/2 + L^2/(8R), h0 + L*(g0+g1)/2] — the Bernstein form
/// is exact.
fn parabola_spline(segment: &ParametricSegment, vitals: &Vitals) -> Spline {
    let (station0, h0) = (segment.start_point[0], segment.start_point[1]);
    let l = vitals.length;
    let g0 = segment.start_gradient;
    let r = vitals.radius;
    // The degree-2 Bernstein coefficients of h(u) = h0 + g0*u + u^2/(2*R):
    // c0 = h0, c1 = h0 + g0*L/2 (the start-tangent line's value at the mid
    // station — NOT the curve's own midpoint), c2 = h0 + L*(g0+g1)/2.
    let mid = h0 + g0 * l * 0.5;
    let end = h0 + g0 * l + l * l / (2.0 * r);
    let local = [(0.0, h0), (l * 0.5, mid), (l, end)];
    let points: Vec<Vector3> = local
        .iter()
        .map(|p| Vector3::new(station0 + p.0, p.1, 0.0))
        .collect();
    let g1 = g0 + l / r;
    let mut spline = Spline::from_control_points(2, points);
    spline.flags.planar = true;
    spline.begin_tangent = Vector3::new(1.0, g0, 0.0);
    spline.end_tangent = Vector3::new(1.0, g1, 0.0);
    spline
}

/// The classical clothoid (Euler spiral / Fresnel) position series — the
/// power expansion of cos/sin(theta) with theta(s) = b*s^2, where
/// b = dkappa/(2*L) = 1/(2*R*L) for a flat-to-R transition (the signed
/// radius included, so a right-hand spiral's y grows negative):
/// x = sum_k (-1)^k b^(2k) s^(4k+1) / ((2k)! (4k+1)),
/// y = sum_k (-1)^k b^(2k+1) s^(4k+3) / ((2k+1)! (4k+3)).
///
/// Used to place the demo chains' welded seams and as the tests'
/// independent oracle against the engine's Gauss-Legendre integrator.
fn fresnel_pos(b: f64, s: f64) -> (f64, f64) {
    fn factorial(n: usize) -> f64 {
        (1..=n as u64).fold(1u128, |acc: u128, v| acc.saturating_mul(v as u128)) as f64
    }
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    for k in 0..12usize {
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        x += sign * b.powi(2 * k as i32)
            * s.powi(4 * k as i32 + 1)
            / (factorial(2 * k) * (4.0 * k as f64 + 1.0));
        y += sign * b.powi(2 * k as i32 + 1)
            * s.powi(4 * k as i32 + 3)
            / (factorial(2 * k + 1) * (4.0 * k as f64 + 3.0));
    }
    (x, y)
}

/// The commission's demo plan chain: LINE + CLOTHOID(R=-300, L=60) + LINE
/// with exact welded seams (the closing line starts at the clothoid's own
/// Fresnel-series end, at its end bearing). The demo fixture for the API
/// surface, the tests and the budget bench.
pub fn commission_demo_plan_chain() -> ParametricChain {
    let l = 60.0f64;
    let big_r = -300.0f64;
    let dkappa = 1.0 / big_r;
    let (x_end, y_end) = fresnel_pos(dkappa / (2.0 * l), l);
    let theta_end = dkappa * l / 2.0;
    ParametricChain {
        plane: ParametricPlane::Plan,
        segments: vec![
            ParametricSegment {
                species: ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
                start_point: [0.0, 0.0],
                start_direction: 0.0,
                start_gradient: 0.0,
                end_gradient: 0.0,
                length: 40.0,
                start_radius: 0.0,
                end_radius: 0.0,
                end_curvature: 0.0,
                bank_change: 0.0,
                gravity_height: 0.0,
            },
            ParametricSegment {
                species: ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Clothoid),
                start_point: [40.0, 0.0],
                start_direction: 0.0,
                start_gradient: 0.0,
                end_gradient: 0.0,
                length: l,
                start_radius: 0.0,
                end_radius: big_r,
                end_curvature: 0.0,
                bank_change: 0.0,
                gravity_height: 0.0,
            },
            ParametricSegment {
                species: ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
                start_point: [40.0 + x_end, y_end],
                start_direction: theta_end,
                start_gradient: 0.0,
                end_gradient: 0.0,
                length: 30.0,
                start_radius: 0.0,
                end_radius: 0.0,
                end_curvature: 0.0,
                bank_change: 0.0,
                gravity_height: 0.0,
            },
        ],
        layer: "CVI_DISPLAY".to_string(),
    }
}

/// The commission's demo profile chain: CONSTANTGRADIENT + PARABOLICARC
/// (g = 0 to +10% over an 80 m run) with the mean-gradient law exact at
/// the seams.
pub fn commission_demo_profile_chain() -> ParametricChain {
    ParametricChain {
        plane: ParametricPlane::Profile,
        segments: vec![
            ParametricSegment {
                species: ParametricSpecies::Vertical(VerticalSegmentSpecies::ConstantGradient),
                start_point: [0.0, 100.0],
                start_direction: 0.0,
                start_gradient: 0.0,
                end_gradient: 0.0,
                length: 60.0,
                start_radius: 0.0,
                end_radius: 0.0,
                end_curvature: 0.0,
                bank_change: 0.0,
                gravity_height: 0.0,
            },
            ParametricSegment {
                species: ParametricSpecies::Vertical(VerticalSegmentSpecies::ParabolicArc),
                start_point: [60.0, 100.0],
                start_direction: 0.0,
                start_gradient: 0.0,
                end_gradient: 0.1,
                length: 80.0,
                start_radius: 800.0,
                end_radius: 0.0,
                end_curvature: 0.0,
                bank_change: 0.0,
                gravity_height: 0.0,
            },
        ],
        layer: "CVI_PROFILE".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// dof-points helper: build a declared plan segment with all defaults.
    fn plan_segment(
        species: ParametricSpecies,
        start: [f64; 2],
        direction: f64,
        length: f64,
        start_radius: f64,
        end_radius: f64,
    ) -> ParametricSegment {
        ParametricSegment {
            species,
            start_point: start,
            start_direction: direction,
            start_gradient: 0.0,
            end_gradient: 0.0,
            length,
            start_radius,
            end_radius,
            end_curvature: 0.0,
            bank_change: 0.0,
            gravity_height: 0.0,
        }
    }

    fn profile_segment(
        species: ParametricSpecies,
        start: [f64; 2],
        length: f64,
        g0: f64,
        g1: f64,
        radius: f64,
        end_curvature: f64,
    ) -> ParametricSegment {
        ParametricSegment {
            species,
            start_point: start,
            start_direction: g0.atan(),
            start_gradient: g0,
            end_gradient: g1,
            length,
            start_radius: radius,
            end_radius: 0.0,
            end_curvature,
            bank_change: 0.0,
            gravity_height: 0.0,
        }
    }

    /// The demo chains live in the module body (the API surface's demo
    /// fixture, shared with the budget bench).
    use super::{commission_demo_plan_chain, commission_demo_profile_chain};

    fn demo_plan_chain() -> ParametricChain {
        commission_demo_plan_chain()
    }

    fn demo_profile_chain() -> ParametricChain {
        commission_demo_profile_chain()
    }

    /// The Fresnel oracle for one flat-to-R clothoid entity at position s.
    fn fresnel_oracle(r: f64, l: f64, s: f64) -> (f64, f64) {
        super::fresnel_pos(1.0 / (2.0 * r * l), s)
    }

    #[test]
    fn plan_line_and_circular_arc_carry_exactly() {
        let chain = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: vec![
                plan_segment(
                    ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
                    [0.0, 0.0],
                    0.0,
                    40.0,
                    0.0,
                    0.0,
                ),
                plan_segment(
                    ParametricSpecies::Horizontal(HorizontalSegmentSpecies::CircularArc),
                    [40.0, 0.0],
                    0.0,
                    45.0, // arc of 0.15 rad on R=300: a gentle sample sweep
                    -300.0,
                    -300.0,
                ),
            ],
            layer: String::new(),
        };
        let (entities, carriers) = build_display_entities(&chain, 1e-3).unwrap();
        assert_eq!(carriers.len(), 2);
        assert!(matches!(carriers[0], StagedCarrier::Exact));
        assert!(matches!(carriers[1], StagedCarrier::Exact));
        assert!(matches!(&entities[0], EntityType::Line(_)));
        assert!(matches!(&entities[1], EntityType::Arc(_)));

        // The line's endpoints are the exact evaluations.
        let samples = [
            sample_chain(&chain, 0.0).unwrap().unwrap(),
            sample_chain(&chain, 40.0).unwrap().unwrap(),
        ];
        assert_eq!(samples[0].point, [0.0, 0.0]);
        assert_eq!(samples[1].point, [40.0, 0.0]);

        // The arc: the mid-point against the exact circle law. The declared
        // arc is CW (R = -300) with an arc length of 45 on |R| 300.
        let mid = sample_chain(&chain, 40.0 + 45.0 / 2.0).unwrap().unwrap();
        let u = 22.5f64;
        let r = -300.0f64;
        let expected_x = 40.0 + r * (u / r).sin();
        let expected_y = r * (1.0 - (u / r).cos());
        assert!((mid.point[0] - expected_x).abs() < 1e-9, "arc x off");
        assert!((mid.point[1] - expected_y).abs() < 1e-9, "arc y off");
    }

    #[test]
    fn parabolic_carrier_is_exact_quadratic_spline() {
        let chain = demo_profile_chain();
        let (entities, carriers) = build_display_entities(&chain, 1e-3).unwrap();
        assert!(matches!(carriers[0], StagedCarrier::Exact));
        assert!(matches!(carriers[1], StagedCarrier::Exact));
        let EntityType::Spline(spline) = &entities[1] else {
            panic!("parabolic segment must stage a spline");
        };
        assert_eq!(spline.degree, 2);
        assert_eq!(spline.control_points.len(), 3);
        assert!(!spline.knots.is_empty(), "clamped knot vector present");

        // Evaluate the degree-2 Bezier at the knot span params and compare
        // with the closed quadratic law h = h0 + g0*u + u^2/(2R).
        let p = |t: f64| {
            let (q0, q1, q2) = (
                spline.control_points[0],
                spline.control_points[1],
                spline.control_points[2],
            );
            let loc = |tt: f64| q0 * (1.0 - tt) * (1.0 - tt) + q1 * 2.0 * tt * (1.0 - tt) + q2 * tt * tt;
            loc(t)
        };
        for t in [0.0f64, 0.25, 0.5, 0.75, 1.0] {
            let u = 80.0 * t;
            let expected = (60.0 + u, 100.0 + 0.0 + u * u / (2.0 * 800.0));
            let got = p(t);
            assert!((got.x - expected.0).abs() < 1e-9, "station off at {t}");
            assert!((got.y - expected.1).abs() < 1e-9, "height off at {t}");
        }

        // The mean-gradient law at the end is EXACT: h(L) = h0 + L(g0+g1)/2.
        let vec_end = p(1.0);
        assert!((vec_end.y - 104.0).abs() < 1e-9, "mean-gradient law must be exact");
    }

    #[test]
    fn cubic_carrier_is_exact_cubic_spline() {
        let chain = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: vec![plan_segment(
                ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Cubic),
                [0.0, 0.0],
                0.0,
                80.0,
                0.0,
                300.0,
            )],
            layer: String::new(),
        };
        let l = 80.0f64;
        let r = 300.0f64;
        let (entities, _) = build_display_entities(&chain, 1e-3).unwrap();
        let EntityType::Spline(spline) = &entities[0] else {
            panic!("cubic must stage a spline");
        };
        assert_eq!(spline.degree, 3);
        assert_eq!(spline.control_points.len(), 4);
        let p = |t: f64| {
            let c = &spline.control_points;
            let b3 = |i: usize, tt: f64| {
                let one_minus = 1.0 - tt;
                match i {
                    0 => one_minus * one_minus * one_minus,
                    1 => 3.0 * tt * one_minus * one_minus,
                    2 => 3.0 * tt * tt * one_minus,
                    _ => tt * tt * tt,
                }
            };
            let mut x = 0.0f64;
            for (i, point) in c.iter().enumerate() {
                x += b3(i, t) * point.x;
            }
            let mut y = 0.0f64;
            for (i, point) in c.iter().enumerate() {
                y += b3(i, t) * point.y;
            }
            (x, y)
        };
        for t in [0.0f64, 0.2, 0.5, 0.8, 1.0] {
            let x = l * t;
            let expected_y = x * x * x / (6.0 * r * l);
            let got = p(t);
            assert!((got.0 - x).abs() < 1e-9, "the x parametrization must stay linear");
            assert!((got.1 - expected_y).abs() < 1e-9, "cubic parabola off at {t}");
        }
    }

    #[test]
    fn gauss_legendre_constants_sum_to_two() {
        // The 16 weights must sum to the full [-1,1] interval's integral of
        // the constant 1: 2.0 (the first debug-cycle caught a mis-entered
        // tail pair; the guard stays).
        let total: f64 = GL16_W.iter().sum();
        assert!((total - 2.0).abs() < 1e-6, "the 16 weights must sum to 2: {total}");
    }

    #[test]
    fn clothoid_matches_fresnel_series() {
        // The engine's Gauss-Legendre integrator against the closed Fresnel
        // series: independent evaluation paths must agree to ~1e-10.
        let l = 60.0f64;
        let r = -300.0f64;
        let chain = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: vec![plan_segment(
                ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Clothoid),
                [0.0, 0.0],
                0.0,
                l,
                0.0,
                r,
            )],
            layer: String::new(),
        };
        for frac in [0.25f64, 0.5, 0.9, 1.0] {
            let u = l * frac;
            let got = sample_chain(&chain, u).unwrap().unwrap();
            let (x_series, y_series) = fresnel_oracle(r, l, u);
            assert!(
                (got.point[0] - x_series).abs() < 1e-9,
                "clothoid x at s={u}: {} vs series {x_series}",
                got.point[0]
            );
            assert!(
                (got.point[1] - y_series).abs() < 1e-9,
                "clothoid y at s={u}: {} vs series {y_series}",
                got.point[1]
            );
        }
        // The end deflection is exactly L/(2R).
        let end = sample_chain(&chain, l).unwrap().unwrap();
        let theta = end.direction[1].atan2(end.direction[0]);
        assert!((theta - l / (2.0 * r)).abs() < 1e-9);
    }

    #[test]
    fn transition_species_stay_within_declared_chord_tolerance() {
        // Tolerance matrix over representative road-curve parameters: the
        // staged chords must never deviate more than the declared tolerance.
        for tolerance in [1e-2f64, 1e-3] {
            for big_r in [300.0f64, 600.0, 1000.0, 2000.0] {
                for l in [40.0f64, 80.0, 200.0] {
                    for species in [
                        HorizontalSegmentSpecies::Clothoid,
                        HorizontalSegmentSpecies::BlossCurve,
                        HorizontalSegmentSpecies::CosineCurve,
                        HorizontalSegmentSpecies::SineCurve,
                        HorizontalSegmentSpecies::HelmertCurve,
                        HorizontalSegmentSpecies::VienneseBend,
                    ] {
                        let chain = ParametricChain {
                            plane: ParametricPlane::Plan,
                            segments: vec![plan_segment(
                                ParametricSpecies::Horizontal(species),
                                [0.0, 0.0],
                                0.0,
                                l,
                                0.0,
                                big_r,
                            )],
                            layer: String::new(),
                        };
                        let (entities, carriers) =
                            build_display_entities(&chain, tolerance).unwrap();
                        let EntityType::LwPolyline(polyline) = &entities[0] else {
                            panic!("blend species must stage chords");
                        };
                        let st = match carriers[0] {
                            StagedCarrier::Chords(count) => count,
                            other => panic!("expected chords, got {other:?}"),
                        };
                        assert!(st <= MAX_PLUGIN_LIVE_ENTITIES, "chord plan within the cap");
                        let eval = |u: f64| {
                            let vitals = validate_segment(chain.plane, &chain.segments[0]).unwrap();
                            evaluate_plan(&species, &chain.segments[0], &vitals, u)
                        };
                        let worst = measured_deviation(polyline, &eval, l);
                        assert!(
                            worst <= tolerance,
                            "{species:?} R={big_r} L={l} t={tolerance}: worst {worst:.3e} m"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn vertical_species_follow_their_laws() {
        // Constant gradient: exact line.
        let chain = demo_profile_chain();
        let sample = sample_chain(&chain, 30.0).unwrap().unwrap();
        assert!((sample.point[1] - 100.0).abs() < 1e-12);

        // Vertical circular arc: exact circle carries the end gradient.
        let r = -500.0f64;
        let l = 90.0f64;
        let g0 = 0.02f64;
        // sin(theta1) = sin(theta0) + L/R determines the consistent g1.
        let s0 = g0.atan().sin();
        let s1 = s0 + l / r;
        let g1 = s1.asin().tan();
        let chain_v = ParametricChain {
            plane: ParametricPlane::Profile,
            segments: vec![profile_segment(
                ParametricSpecies::Vertical(VerticalSegmentSpecies::CircularArc),
                [100.0, 105.0],
                l,
                g0,
                g1,
                r,
                0.0,
            )],
            layer: String::new(),
        };
        let eval_u = 45.0f64;
        let vitals = validate_segment(chain_v.plane, &chain_v.segments[0]).unwrap();
        let sample = evaluate_segment_at(chain_v.plane, &chain_v.segments[0], &vitals, eval_u);
        let theta = (g0.atan().sin() + eval_u / r).asin();
        assert!(
            (sample.point[1] - (105.0 + r * (g0.atan().cos() - theta.cos()))).abs() < 1e-9,
            "the vertical circle's exact height law"
        );

        // Vertical clothoid: gradient ends realized by the declared
        // curvatures (the record model's law once it carries them).
        let g0 = -0.05f64;
        let g1 = 0.05f64;
        let l = 100.0f64;
        let dtheta = g1.atan() - g0.atan();
        let end_curvature = 2.0 * dtheta / l;
        let chain_vc = ParametricChain {
            plane: ParametricPlane::Profile,
            segments: vec![profile_segment(
                ParametricSpecies::Vertical(VerticalSegmentSpecies::Clothoid),
                [0.0, 10.0],
                l,
                g0,
                g1,
                0.0,
                end_curvature,
            )],
            layer: String::new(),
        };
        let sample = sample_chain(&chain_vc, l).unwrap().unwrap();
        let realized = sample.direction[1] / sample.direction[0];
        assert!((realized - g1).abs() < 1e-6, "the end gradient is realized exactly");
    }

    #[test]
    fn profile_arc_entity_draws_the_run_law_angular_span() {
        // The staged vertical circle's ARC must span exactly the run law's
        // angular interval: sweep = |theta1 - theta0| where
        // sin(theta1) = sin(theta0) + L/R — NOT L/|R|, which falls short of
        // the declared end point for every real gradient pair.
        for r in [800.0f64, -800.0] {
            let l = 200.0f64;
            let g0 = 0.04f64;
            let s0 = g0.atan().sin();
            let s1 = s0 + l / r;
            assert!(s1.abs() < 1.0, "fixture stays in the sloped range");
            let g1 = s1.asin().tan();
            let chain = ParametricChain {
                plane: ParametricPlane::Profile,
                segments: vec![profile_segment(
                    ParametricSpecies::Vertical(VerticalSegmentSpecies::CircularArc),
                    [100.0, 105.0],
                    l,
                    g0,
                    g1,
                    r,
                    0.0,
                )],
                layer: String::new(),
            };
            let (entities, carriers) = build_display_entities(&chain, 1e-3).unwrap();
            assert!(matches!(carriers[0], StagedCarrier::Exact));
            let EntityType::Arc(arc) = &entities[0] else {
                panic!("the vertical circle stages an arc");
            };
            let theta0 = g0.atan();
            let theta1 = s1.asin();
            assert_eq!(arc.radius, r.abs());
            let expected_sweep = (theta1 - theta0).abs();
            assert!(
                (arc.sweep_angle() - expected_sweep).abs() < 1e-9,
                "the drawn sweep {} must be the run law's angular span {} on R={r}",
                arc.sweep_angle(),
                expected_sweep
            );
            // The entity's drawn endpoints are the law's segment endpoints
            // (start-side for CCW positive radii, swapped for CW negative).
            let law_start = (100.0f64, 105.0f64);
            let law_end = (100.0 + l, 105.0 + r * (theta0.cos() - theta1.cos()));
            let (begin, finish) = if r > 0.0 {
                (law_start, law_end)
            } else {
                (law_end, law_start)
            };
            assert!((arc.start_point().x - begin.0).abs() < 1e-9, "begin x R={r}");
            assert!((arc.start_point().y - begin.1).abs() < 1e-9, "begin y R={r}");
            assert!((arc.end_point().x - finish.0).abs() < 1e-9, "finish x R={r}");
            assert!((arc.end_point().y - finish.1).abs() < 1e-9, "finish y R={r}");
            // And the drawn midpoint rides the same circle.
            let mid = arc.midpoint();
            let radial = ((mid.x - arc.center.x).powi(2) + (mid.y - arc.center.y).powi(2)).sqrt();
            assert!((radial - r.abs()).abs() < 1e-6, "the drawn midpoint leaves the circle");
        }
    }

    #[test]
    fn vertical_clothoid_without_curvature_ends_is_refused() {
        let chain = ParametricChain {
            plane: ParametricPlane::Profile,
            segments: vec![profile_segment(
                ParametricSpecies::Vertical(VerticalSegmentSpecies::Clothoid),
                [0.0, 10.0],
                100.0,
                -0.05,
                0.05,
                0.0,
                0.0,
            )],
            layer: String::new(),
        };
        let err = build_display_entities(&chain, 1e-3).unwrap_err();
        assert!(err.contains("curvature ends"), "{err}");
    }

    #[test]
    fn demo_chains_audit_green_and_sample_continuous() {
        let plan = demo_plan_chain();
        audit_chain(&plan).expect("the demo plan chain audits (seams exact)");
        let profile = demo_profile_chain();
        audit_chain(&profile)
            .expect("the demo profile chain audits (the mean-gradient seam is exact)");

        // The samples weld at the seams: hovering either epsilon side of the
        // LINE->CLOTHOID boundary answers the same welded point.
        let acc0 = plan.segments[0].length;
        let end0 = sample_chain(&plan, acc0 - 1e-9).unwrap().unwrap();
        let start1 = sample_chain(&plan, acc0 + 1e-9).unwrap().unwrap();
        assert!(
            ((end0.point[0] - start1.point[0]).abs() + (end0.point[1] - start1.point[1]).abs())
                < 1e-6,
            "the sample welds at the LINE->CLOTHOID seam"
        );

        // The chain end samples the declared tail's end exactly: the third
        // segment (a line of L=30) extends its declared start by 30*dir.
        let total: f64 = plan.segments.iter().map(|s| s.length).sum();
        let tail = sample_chain(&plan, total).unwrap().unwrap();
        let start = plan.segments[2].start_point;
        let dir = plan.segments[2].start_direction;
        let expected = [start[0] + 30.0 * dir.cos(), start[1] + 30.0 * dir.sin()];
        assert!((tail.point[0] - expected[0]).abs() < 1e-9, "tail x");
        assert!((tail.point[1] - expected[1]).abs() < 1e-9, "tail y");
    }

    #[test]
    fn sample_chain_outside_answers_none() {
        let chain = demo_profile_chain();
        assert!(sample_chain(&chain, -0.1).unwrap().is_none());
        let total: f64 = chain.segments.iter().map(|s| s.length).sum();
        assert!(sample_chain(&chain, total + 1e-3).unwrap().is_none());
        assert!(sample_chain(&chain, f64::NAN).is_err());
        assert!(sample_chain(&chain, f64::INFINITY).is_err());
    }

    #[test]
    fn seam_refusal_names_the_pair() {
        let mut chain = demo_plan_chain();
        // Break the seam: the second line starts 1e-4 m away from the
        // clothoid's evaluated end.
        chain.segments[2].start_point[0] += 1e-4;
        let err = build_display_entities(&chain, 1e-3).unwrap_err();
        assert!(err.contains("seam segment 1 to 2"), "names the audited pair: {err}");
        assert!(err.contains("not continuous"), "{err}");

        // A tangential break refuses too: flip the second line's direction.
        let mut chain = demo_plan_chain();
        chain.segments[2].start_direction = -0.1 + 1e-4;
        let err = build_display_entities(&chain, 1e-3).unwrap_err();
        assert!(
            err.contains("tangentially continuous"),
            "the direction break is named: {err}"
        );
    }

    #[test]
    fn viennese_without_cant_matches_the_quintic_law() {
        // Without the cant coupling (bank_change = gravity_height = 0) the
        // Viennese bend evaluates just its quintic base law: the deflection
        // ends at L/(2*R) and the halfway deflection is W(1/2) of the
        // quintic integral (0.06835...).
        let chain = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: vec![plan_segment(
                ParametricSpecies::Horizontal(HorizontalSegmentSpecies::VienneseBend),
                [0.0, 0.0],
                0.0,
                100.0,
                0.0,
                -400.0,
            )],
            layer: String::new(),
        };
        let vitals = validate_segment(chain.plane, &chain.segments[0]).unwrap();
        let species = HorizontalSegmentSpecies::VienneseBend;
        let theta_end = plan_local_theta(&species, &vitals, 100.0);
        assert!((theta_end - (100.0 / (2.0 * -400.0))).abs() < 1e-12);
        let theta_mid = plan_local_theta(&species, &vitals, 50.0);
        let w_mid = blend_integral(species, 0.5);
        assert!(
            (theta_mid - w_mid * (1.0 / -400.0) * 100.0).abs() < 1e-12,
            "the halfway integral is the quintic's own W(1/2)"
        );
    }

    #[test]
    fn helmert_blends_the_two_quadratic_halves() {
        let chain = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: vec![plan_segment(
                ParametricSpecies::Horizontal(HorizontalSegmentSpecies::HelmertCurve),
                [0.0, 0.0],
                0.0,
                120.0,
                0.0,
                -500.0,
            )],
            layer: String::new(),
        };
        let vitals = validate_segment(chain.plane, &chain.segments[0]).unwrap();
        let species = HorizontalSegmentSpecies::HelmertCurve;
        // The piecewise halves join continuously at the mid: theta(L/2) =
        // dTheta/6 for the flat-to-R law (W(1/2) = 1/12 integrates to a
        // sixth of the total L/(2R) deflection).
        let theta_mid = plan_local_theta(&species, &vitals, 60.0);
        assert!((theta_mid - (120.0 / (2.0 * -500.0)) / 6.0).abs() < 1e-9);
        // W(1) = 1/2: the end deflection is L/(2R) exactly.
        let theta_end = plan_local_theta(&species, &vitals, 120.0);
        assert!((theta_end - 120.0 / (2.0 * -500.0)).abs() < 1e-12);
    }

    #[test]
    fn cap_refuses_over_budget_chains() {
        // A 50-segment chain (short lines welded to clothoids, seams exact
        // by construction) at a 1e-4 tolerance: the chord plans sum far
        // above the live cap, so nothing publishes.
        let mut segments = Vec::new();
        let mut cursor = [0.0f64, 0.0];
        let mut heading = 0.0f64;
        for _ in 0..25 {
            segments.push(plan_segment(
                ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
                cursor,
                heading,
                2.0,
                0.0,
                0.0,
            ));
            cursor = [
                cursor[0] + 2.0 * heading.cos(),
                cursor[1] + 2.0 * heading.sin(),
            ];
            segments.push(plan_segment(
                ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Clothoid),
                cursor,
                heading,
                60.0,
                0.0,
                -300.0,
            ));
            let dtheta = 60.0 / (2.0 * -300.0);
            let end = {
                let chain = ParametricChain {
                    plane: ParametricPlane::Plan,
                    segments: vec![segments.last().unwrap().clone()],
                    layer: String::new(),
                };
                let vitals = audit_chain(&chain).unwrap();
                evaluate_segment_at(chain.plane, &chain.segments[0], &vitals[0], 60.0)
            };
            cursor = end.point;
            heading += dtheta;
        }
        let chain = ParametricChain {
            plane: ParametricPlane::Plan,
            segments,
            layer: String::new(),
        };
        audit_chain(&chain).expect("the welded 50-segment chain audits");
        let result = build_display_entities(&chain, 1e-4);
        let err = result.err().expect("the chain's footprint is over the cap");
        assert!(err.contains("MAX_PLUGIN_LIVE_ENTITIES"), "{err}");
        assert!(err.contains("nothing staged"), "{err}");
    }

    #[test]
    fn helix_never_stages_a_plan_clothoid() {
        // The contract's negative: a plan clothoid is a 2D curve with linear
        // curvature; a Helix is a 3D constant-pitch space spiral. The staged
        // carrier set only ever contains the four supported types.
        let chain = demo_plan_chain();
        let (entities, _) = build_display_entities(&chain, 1e-3).unwrap();
        for entity in &entities {
            assert!(
                matches!(
                    entity,
                    EntityType::Line(_)
                        | EntityType::Arc(_)
                        | EntityType::LwPolyline(_)
                        | EntityType::Spline(_)
                ),
                "only line/arc/spline/polyline carriers exist in this contract"
            );
        }
    }

    #[test]
    fn staged_chain_and_xrecords_survive_dxf_and_dwg_round_trips() {
        // THE entity contract of this surface: the staged display entities
        // ride the shared drawing model, and the record model's XRecords —
        // the plugins' declared truth — survive the SAME file round trip.
        // Loss reports must stay empty for both formats.
        use codec::Format;

        // 1) Declare + stage.
        let plan = demo_plan_chain();
        let profile = demo_profile_chain();
        let mut all_entities = Vec::new();
        all_entities.extend(build_display_entities(&plan, 1e-3).unwrap().0);
        all_entities.extend(build_display_entities(&profile, 1e-3).unwrap().0);
        // One staged entity carries the record model's XDATA (the placement
        // class the entity-bound records ride in).
        if let EntityType::LwPolyline(polyline) = &mut all_entities[1] {
            let mut record = codec::xdata::ExtendedDataRecord::new("CVIFC");
            record.add_value(codec::xdata::XDataValue::String("alignment display carrier".into()));
            record.add_value(codec::xdata::XDataValue::Real(104.0));
            polyline.common.extended_data.add_record(record);
        }

        // 2) A document + the staged set + the record model's XRecords.
        let mut document = codec::Document::new();
        // 2b) The display layers are registered entries of the document's
        // layer table (the plugin consumer creates its display layers
        // before staging; an unregistered layer name falls back to "0" on
        // read, by the model's own contract).
        document
            .layers
            .add(codec::tables::Layer::new("CVI_DISPLAY"))
            .unwrap();
        document
            .layers
            .add(codec::tables::Layer::new("CVI_PROFILE"))
            .unwrap();
        let mut staged_handles = Vec::new();
        for entity in all_entities {
            staged_handles.push(document.add_entity(entity).unwrap());
        }
        let appid = codec::tables::AppId::new("CVIFC");
        document.app_ids.add(appid).unwrap();
        let nod_handle = document.header.named_objects_dict_handle;
        let dict_handle = document.allocate_handle();
        let record_handle = document.allocate_handle();
        let mut record = codec::objects::XRecord::named("CVIFC_ALIGN_DISPLAY");
        record.handle = record_handle;
        record.owner = dict_handle;
        record.entries = vec![
            codec::objects::XRecordEntry::string(1, "parametric chain v14"),
            codec::objects::XRecordEntry::double(140, 104.0),
            codec::objects::XRecordEntry::int32(420, 7),
        ];
        document
            .objects
            .insert(record_handle, codec::objects::ObjectType::XRecord(record));
        let mut dict = codec::objects::Dictionary::new();
        dict.handle = dict_handle;
        dict.owner = nod_handle;
        dict.hard_owner = true;
        // The record's name and its dictionary key are ONE name (the DWG
        // canonical semantics: the XRecord's identity rides the owning
        // dictionary).
        dict.add_entry("CVIFC_ALIGN_DISPLAY", record_handle);
        document
            .objects
            .insert(dict_handle, codec::objects::ObjectType::Dictionary(dict));
        if let Some(codec::objects::ObjectType::Dictionary(nod)) = document.objects.get_mut(&nod_handle) {
            nod.add_entry("CVIFC", dict_handle);
        }

        // 3) Round trips.
        let original_record =
            document.objects.get(&record_handle).cloned().expect("record present");
        let original_kind_of = |h: codec::Handle| {
            document.get_entity(h).map(|e| std::mem::discriminant(e))
        };
        let expected_kinds: Vec<_> = staged_handles
            .iter()
            .map(|h| original_kind_of(*h).expect("handle present"))
            .collect();

        let (dxf_bytes, dxf_losses) = codec::Dxf.write(&document).unwrap();
        assert!(dxf_losses.is_empty(), "DXF write must not lose anything: {dxf_losses:?}");
        let (dxf_doc, dxf_read_losses) = codec::Dxf.read(&dxf_bytes).unwrap();
        assert!(
            dxf_read_losses.is_empty(),
            "DXF read must not lose anything: {dxf_read_losses:?}"
        );
        assert_staged(&dxf_doc, &staged_handles, &expected_kinds);
        assert_record_model(&dxf_doc, &original_record);

        let (dwg_bytes, dwg_losses) = codec::Dwg.write(&document).unwrap();
        assert!(dwg_losses.is_empty(), "DWG write must not lose anything: {dwg_losses:?}");
        let (dwg_doc, dwg_read_losses) = codec::Dwg.read(&dwg_bytes).unwrap();
        assert!(
            dwg_read_losses.is_empty(),
            "DWG read must not lose anything: {dwg_read_losses:?}"
        );
        assert_staged(&dwg_doc, &staged_handles, &expected_kinds);
        assert_record_model(&dwg_doc, &original_record);

        // 4) The entity-bound record rides too: the staged polyline's XDATA
        // (the CVIFC app record) survives, values verbatim.
        let assert_xdata = |document: &codec::Document, stage: &str| {
            let entity = document
                .get_entity(staged_handles[1])
                .expect("staged polyline present");
            if let EntityType::LwPolyline(polyline) = entity {
                let record = polyline
                    .common
                    .extended_data
                    .get_record("CVIFC")
                    .unwrap_or_else(|| panic!("{stage}: the CVIFC XDATA lost"));
                assert_eq!(record.values.len(), 2);
            } else {
                panic!("{stage}: entity 1 became another type");
            }
        };
        assert_xdata(&dxf_doc, "DXF");
        assert_xdata(&dwg_doc, "DWG");

        // 5) The second hop (file -> document -> file) must be stationary.
        let (dxf_bytes2, dxf_losses2) = codec::Dxf.write(&dxf_doc).unwrap();
        assert!(dxf_losses2.is_empty());
        let (dxf_doc2, _) = codec::Dxf.read(&dxf_bytes2).unwrap();
        assert_record_model(&dxf_doc2, &original_record);
        assert_staged(&dxf_doc2, &staged_handles, &expected_kinds);

        fn assert_staged(
            document: &codec::Document,
            staged: &[codec::Handle],
            kinds: &[std::mem::Discriminant<codec::EntityType>],
        ) {
            for (i, (handle, kind)) in staged.iter().zip(kinds.iter()).enumerate() {
                let entity = document
                    .get_entity(*handle)
                    .unwrap_or_else(|| panic!("staged entity {handle:?} lost in the round trip"));
                assert_eq!(std::mem::discriminant(entity), *kind, "type drifted");
                // The chain's layer attribution survives the file hop: the
                // demo plan stages on CVI_DISPLAY, the demo profile on
                // CVI_PROFILE.
                let expected = if i < 3 { "CVI_DISPLAY" } else { "CVI_PROFILE" };
                assert_eq!(entity.common().layer, expected, "layer attribution lost");
            }
        }

        fn assert_record_model(document: &codec::Document, original: &codec::objects::ObjectType) {
            // The XRecord's entries must survive BYTE-IDENTICAL, alongside
            // its dictionary and the NOD linkage (record + display live and
            // die together in the same file).
            let codec::objects::ObjectType::XRecord(original_record) = original else {
                panic!("expected an XRecord");
            };
            let read = document
                .objects
                .get(&original_record.handle)
                .unwrap_or_else(|| panic!("XRecord lost in the round trip"));
            let codec::objects::ObjectType::XRecord(read_record) = read else {
                panic!("XRecord became another object");
            };
            assert_eq!(read_record.entries, original_record.entries, "entries must be byte-identical");
            assert_eq!(read_record.name, original_record.name);
            // The extension dictionary survived with its entries.
            let dict_read = document
                .objects
                .get(&original_record.owner)
                .expect("the record's dictionary survived");
            let codec::objects::ObjectType::Dictionary(dict) = dict_read else {
                panic!("dictionary became another object");
            };
            assert!(
                dict.entries
                    .iter()
                    .any(|(name, _)| name == "CVIFC_ALIGN_DISPLAY"),
                "the dictionary entry survived"
            );
        }
    }

    #[test]
    fn entity_layers_ride_the_chain_payload() {
        let chain = demo_plan_chain();
        let (entities, _) = build_display_entities(&chain, 1e-3).unwrap();
        for entity in &entities {
            assert_eq!(entity.common().layer, "CVI_DISPLAY");
        }
        let mut chain = demo_plan_chain();
        chain.layer = String::new();
        let (entities, _) = build_display_entities(&chain, 1e-3).unwrap();
        for entity in &entities {
            // An empty layer name keeps the codec default.
            assert_eq!(entity.common().layer, "0");
        }
    }

    #[test]
    fn grip_edits_rederive_within_the_audited_contract() {
        // The grip-edit correctness arc (engine level): a grip drag revises
        // one DECLARED parameter, the plugin's honest re-declare propagates
        // the moved JOINT from the engine's own sampling (no duplicated
        // math), and the audited chain law holds on the edited set before
        // any carrier is rebuilt. No timing asserts — the bench owns the
        // budgets; this pins the edit -> re-derive -> audit -> carrier path.

        // ── the vertical crest fixture ────────────────────────────────
        // [CONSTANTGRADIENT g0=0.01 L=100 → PARABOLICARC g0=0.01 g1=0.06
        // L=200 (R=4000) → CONSTANTGRADIENT g0=0.06 L=80 at the exact end
        // (300, 8.0)], the chain entering at (0, 0).
        let vertical = ParametricChain {
            plane: ParametricPlane::Profile,
            segments: vec![
                profile_segment(
                    ParametricSpecies::Vertical(VerticalSegmentSpecies::ConstantGradient),
                    [0.0, 0.0],
                    100.0,
                    0.01,
                    0.01,
                    0.0,
                    0.0,
                ),
                profile_segment(
                    ParametricSpecies::Vertical(VerticalSegmentSpecies::ParabolicArc),
                    [100.0, 1.0],
                    200.0,
                    0.01,
                    0.06,
                    4000.0,
                    0.0,
                ),
                profile_segment(
                    ParametricSpecies::Vertical(VerticalSegmentSpecies::ConstantGradient),
                    [300.0, 8.0],
                    80.0,
                    0.06,
                    0.06,
                    0.0,
                    0.0,
                ),
            ],
            layer: String::new(),
        };
        audit_chain(&vertical).expect("the crest fixture audits (the mean-gradient seams are exact)");

        // THE DRAG: the crest's end gradient 0.06 -> 0.09, so the honest
        // vertex radius R = L/(g1-g0) = 2500. The JOINT moved; the stale
        // downstream start is refused — the chain-level law a drag cannot
        // dodge, exactly the check the plugin's records ride.
        let mut edited = vertical.clone();
        edited.segments[1].end_gradient = 0.09;
        edited.segments[1].start_radius = edited.segments[1].length / (0.09 - 0.01);
        let err = audit_chain(&edited).unwrap_err();
        assert!(
            err.contains("seam segment 1 to 2"),
            "the stale joint refuses with the audited pair: {err}"
        );

        // The plugin's honest re-declare: the propagated joint comes from
        // sample_chain at the edited front chain's end (s = 300) — the
        // engine's own evaluation, never a re-typed constant.
        let front = ParametricChain {
            plane: ParametricPlane::Profile,
            segments: edited.segments[..2].to_vec(),
            layer: String::new(),
        };
        let joint = sample_chain(&front, 300.0)
            .expect("the front samples")
            .expect("the joint sits at the edited front chain's end");
        assert!((joint.point[0] - 300.0).abs() < 1e-9);
        assert!(
            (joint.point[1] - 11.0).abs() < 1e-9,
            "the crest height moved to the new mean-gradient end ({})",
            joint.point[1]
        );
        let joint_gradient = joint.direction[1] / joint.direction[0];
        assert!(
            (joint_gradient - 0.09).abs() < 1e-9,
            "the joint slope restates the edited end gradient"
        );
        edited.segments[2].start_point = joint.point;
        edited.segments[2].start_gradient = joint_gradient;
        edited.segments[2].end_gradient = joint_gradient;
        edited.segments[2].start_direction = joint_gradient.atan();
        audit_chain(&edited).expect("the honest re-declare audits green");

        // The drag propagates the whole way: the tail's end rides the moved
        // joint (h = 11 + 0.09*80 = 18.2).
        let tail_end = sample_chain(&edited, 380.0).expect("the edited chain samples").unwrap();
        assert!(
            (tail_end.point[1] - (11.0 + 0.09 * 80.0)).abs() < 1e-9,
            "the edited tail's end follows the moved joint"
        );

        // The carrier path follows: three exact carriers again, and the
        // crest spline's last control point rides the new crest height.
        let (entities, carriers) = build_display_entities(&edited, 1e-3).expect("the edited crest stages");
        assert_eq!(entities.len(), 3);
        assert!(matches!(carriers[0], StagedCarrier::Exact));
        assert!(matches!(carriers[1], StagedCarrier::Exact));
        assert!(matches!(carriers[2], StagedCarrier::Exact));
        let EntityType::Spline(spline) = &entities[1] else {
            panic!("the crest stages an exact degree-2 spline");
        };
        assert_eq!(spline.degree, 2);
        let crest = &spline.control_points[2];
        assert!((crest.x - 300.0).abs() < 1e-9);
        assert!(
            (crest.y - 11.0).abs() < 1e-9,
            "the crest control point rides the edited height"
        );

        // REFUSAL 1 — the degenerate parabola: driving g1 back to g0 makes
        // the honest vertex radius L/(g1-g0) infinite; the engine refuses
        // the non-finite declared field instead of staging a phantom
        // species (the honest solver's division by zero must stop at the
        // validation gate).
        let mut degenerate = vertical.clone();
        degenerate.segments[1].end_gradient = degenerate.segments[1].start_gradient;
        let flat_delta = degenerate.segments[1].end_gradient - degenerate.segments[1].start_gradient;
        degenerate.segments[1].start_radius = degenerate.segments[1].length / flat_delta;
        let err = audit_chain(&degenerate).unwrap_err();
        assert!(
            err.contains("not finite"),
            "the degenerate crest refuses at the validation gate: {err}"
        );

        // REFUSAL 2 — the out-of-range vertical drag: the sheet plane's run
        // law cannot carry a drag whose radius drives the realized slope
        // past the sloped range (sin(theta1) >= 1). This is the vertical
        // family's own range law (the circular run: sin(theta1) =
        // sin(theta0) + L/R); the parabola's and the vertical clothoid's
        // atan-based slope laws have no range to leave.
        let out_of_range = ParametricChain {
            plane: ParametricPlane::Profile,
            segments: vec![profile_segment(
                ParametricSpecies::Vertical(VerticalSegmentSpecies::CircularArc),
                [100.0, 1.0],
                200.0,
                0.01,
                0.99,
                150.0,
                0.0,
            )],
            layer: String::new(),
        };
        let err = audit_chain(&out_of_range).unwrap_err();
        assert!(
            err.contains("leaves the sloped range"),
            "the out-of-range vertical drag refuses: {err}"
        );

        // ── the plan ramp mirror ──────────────────────────────────────
        // [LINE 60 → CLOTHOID R=-300 L=60 → LINE 40], the closing line
        // starting at the clothoid's own Fresnel-series end (the demo
        // chains' welded-seam arithmetic).
        let l = 60.0f64;
        let big_r = -300.0f64;
        let dkappa = 1.0 / big_r;
        let (x_end, y_end) = fresnel_pos(dkappa / (2.0 * l), l);
        let theta_end = dkappa * l / 2.0;
        let ramp = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: vec![
                plan_segment(
                    ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
                    [0.0, 0.0],
                    0.0,
                    60.0,
                    0.0,
                    0.0,
                ),
                plan_segment(
                    ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Clothoid),
                    [60.0, 0.0],
                    0.0,
                    l,
                    0.0,
                    big_r,
                ),
                plan_segment(
                    ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
                    [60.0 + x_end, y_end],
                    theta_end,
                    40.0,
                    0.0,
                    0.0,
                ),
            ],
            layer: String::new(),
        };
        audit_chain(&ramp).expect("the ramp fixture audits");

        // THE DRAG (mirror): the clothoid's end radius -300 -> -240. The
        // joint moved; the stale downstream start refuses, same law.
        let mut edited_ramp = ramp.clone();
        edited_ramp.segments[1].end_radius = -240.0;
        let err = audit_chain(&edited_ramp).unwrap_err();
        assert!(
            err.contains("seam segment 1 to 2"),
            "the stale ramp joint refuses: {err}"
        );

        // The honest propagation: the sampled (Fresnel-law) end of the
        // edited front chain re-anchors the tail, and the sample is
        // cross-checked against the series once.
        let front_ramp = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: edited_ramp.segments[..2].to_vec(),
            layer: String::new(),
        };
        let ramp_joint = sample_chain(&front_ramp, 120.0).expect("the front samples").unwrap();
        let dk2 = 1.0 / -240.0f64;
        let (fx, fy) = fresnel_pos(dk2 / (2.0 * l), l);
        assert!(
            (ramp_joint.point[0] - (60.0 + fx)).abs() < 1e-6,
            "the sampled end restates the Fresnel end (x): {} vs {}",
            ramp_joint.point[0],
            60.0 + fx
        );
        assert!((ramp_joint.point[1] - fy).abs() < 1e-6, "the sampled end restates the Fresnel end (y)");
        assert!(
            (ramp_joint.direction[1].atan2(ramp_joint.direction[0]) - dk2 * l / 2.0).abs() < 1e-9,
            "the end bearing is the exact L/(2R) deflection"
        );
        edited_ramp.segments[2].start_point = ramp_joint.point;
        edited_ramp.segments[2].start_direction =
            ramp_joint.direction[1].atan2(ramp_joint.direction[0]);
        audit_chain(&edited_ramp).expect("the plan mirror's honest re-declare audits green");

        // The edited plan set rebuilds: the same carrier SHAPES (line,
        // chords, line) with the new chord plan measuring within its
        // declared tolerance — the carrier path for the transcendental
        // species follows the edit too.
        let (ramp_entities, ramp_carriers) =
            build_display_entities(&edited_ramp, 1e-3).expect("the edited ramp stages");
        assert_eq!(ramp_entities.len(), 3);
        assert!(matches!(ramp_carriers[0], StagedCarrier::Exact));
        assert!(matches!(ramp_carriers[1], StagedCarrier::Chords(_)));
        assert!(matches!(ramp_carriers[2], StagedCarrier::Exact));
    }

    #[test]
    fn declared_ramp_restates_every_blend_species() {
        // The declared vocabulary must cover the frozen IFC blend set
        // exactly: each blend species restated as (ramp, k0 = 1/start_radius,
        // k1 = end_curvature) evaluates IDENTICALLY through the full
        // evaluation path (the species and the Declared head share the one
        // formula table; a drift between them breaks this at 1e-12).
        let cases: [(HorizontalSegmentSpecies, DeclaredKappaRamp); 6] = [
            (
                HorizontalSegmentSpecies::Clothoid,
                DeclaredKappaRamp::Linear,
            ),
            (
                HorizontalSegmentSpecies::BlossCurve,
                DeclaredKappaRamp::Bloss,
            ),
            (
                HorizontalSegmentSpecies::CosineCurve,
                DeclaredKappaRamp::Cosine,
            ),
            (
                HorizontalSegmentSpecies::SineCurve,
                DeclaredKappaRamp::Sine,
            ),
            (
                HorizontalSegmentSpecies::HelmertCurve,
                DeclaredKappaRamp::Helmert,
            ),
            (
                HorizontalSegmentSpecies::VienneseBend,
                DeclaredKappaRamp::Viennese,
            ),
        ];
        for (hardcoded, ramp) in cases {
            // A straight-entry, circle-exit blend: the hard-coded law reads
            // k1 from the end radius, the Declared law states it directly
            // (the same value either way).
            let mut frozen = plan_segment(
                ParametricSpecies::Horizontal(hardcoded),
                [0.0, 0.0],
                0.0,
                60.0,
                0.0,
                -300.0,
            );
            frozen.end_curvature = 1.0 / frozen.end_radius;
            let mut declared = plan_segment(
                ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Declared(ramp)),
                [0.0, 0.0],
                0.0,
                60.0,
                0.0,
                0.0,
            );
            declared.end_curvature = 1.0 / -300.0;
            for k in 0..=20usize {
                let u = 60.0 * k as f64 / 20.0;
                let frozen_sample = evaluate_plan(&hardcoded, &frozen, &vitals_of(&frozen), u);
                let declared_sample = evaluate_plan(
                    &HorizontalSegmentSpecies::Declared(ramp),
                    &declared,
                    &vitals_of(&declared),
                    u,
                );
                for axis in 0..2 {
                    assert!(
                        (frozen_sample.point[axis] - declared_sample.point[axis]).abs() < 1e-12,
                        "{ramp:?} drifts from {hardcoded:?} (point axis {axis} at u = {u})"
                    );
                    assert!(
                        (frozen_sample.direction[axis] - declared_sample.direction[axis]).abs()
                            < 1e-12,
                        "{ramp:?} drifts from {hardcoded:?} (direction axis {axis} at u = {u})"
                    );
                }
            }
        }
    }

    #[test]
    fn declared_piecewise_blend_stages_end_to_end() {
        // The genuinely-custom primitive: a piecewise-linear ramp through
        // three knots declares a blend the host does not hard-code; it
        // audits, stages chords within the tolerance and samples per the
        // law's own end heading.
        let mut knots = [(0.0, 0.0); 6];
        knots[0] = (0.0, 0.0);
        knots[1] = (0.5, 1.0);
        knots[2] = (1.0, 1.0);
        let mut segment = plan_segment(
            ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Declared(
                DeclaredKappaRamp::Piecewise { count: 3, knots },
            )),
            [0.0, 0.0],
            0.0,
            60.0,
            0.0,
            0.0,
        );
        segment.end_curvature = 1.0 / 500.0;
        audit_chain(&single_chain(ParametricPlane::Plan, &segment)).expect("the declared law audits");
        let (entities, carriers) =
            build_display_entities(&single_chain(ParametricPlane::Plan, &segment), 1e-3).expect("the declared law stages");
        assert_eq!(entities.len(), 1);
        assert!(matches!(carriers[0], StagedCarrier::Chords(_)));
        // The law's own end heading: theta(1) = k0*L + (k1-k0)*L*W(1) with
        // THIS table's W(1) = 0.25 (the ramp 0->1 over half the run) +
        // 0.5 (the flat 1 over the other half) = 0.75 — a custom ramp's
        // deflection is its own integral (only the endpoint VALUE is the
        // declared k1 = 1/500; the 1/2 normalization is the IFC species'
        // own property). theta(1) = 60 * (1/500) * 0.75 = 0.09 rad.
        let sample =
            sample_chain(&single_chain(ParametricPlane::Plan, &segment), 60.0).unwrap().unwrap();
        assert!(
            (sample.direction[1].atan2(sample.direction[0]) - 0.09).abs() < 1e-9
        );
    }

    #[test]
    fn declared_vertical_blend_reads_the_sheet_law() {
        // The vertical Declared head: the straight-run entry (start_radius
        // = 0), the declared end curvature (the vertical clothoid's record
        // rule verbatim), the theta run law and the height integral ride
        // the generic executor; the seamed chain audits end to end.
        let mut blend = profile_segment(
            ParametricSpecies::Vertical(VerticalSegmentSpecies::Declared(
                DeclaredKappaRamp::Sine,
            )),
            [0.0, 0.0],
            100.0,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        blend.end_curvature = 0.004;
        let front = single_chain(ParametricPlane::Profile, &blend);
        let joint = sample_chain(&front, 100.0).unwrap().unwrap();
        // theta(1) = k0*L + dk*L*W(1) = 0.004 * 100 * 0.5 = 0.2 rad.
        let end_gradient = joint.direction[1] / joint.direction[0];
        assert!((end_gradient - 0.2f64.tan()).abs() < 1e-9);
        blend.end_gradient = end_gradient;
        // Chain it into a constant-gradient tail via the propagation
        // idiom, audit green.
        let tail = profile_segment(
            ParametricSpecies::Vertical(VerticalSegmentSpecies::ConstantGradient),
            joint.point,
            80.0,
            end_gradient,
            end_gradient,
            0.0,
            0.0,
        );
        let chain = ParametricChain {
            plane: ParametricPlane::Profile,
            segments: vec![blend, tail],
            layer: String::new(),
        };
        audit_chain(&chain).expect("the declared vertical chain audits");
        assert!(
            matches!(
                build_display_entities(&chain, 1e-3).expect("stages").1[0],
                StagedCarrier::Chords(_)
            ),
            "the declared vertical blend stages chords"
        );
    }

    #[test]
    fn declared_vertical_blend_height_follows_the_ramp() {
        // REGRESSION (the fresh-eyes review): the height must integrate the
        // DECLARED heading's slope — a ramp-blind integrand restated the
        // clothoid's height (~1 m off at the end for this fixture), which no
        // heading-only pin could catch. The oracle integrates the realized
        // slope independently (a fine trapezoid over the closed-form law).
        let mut blend = profile_segment(
            ParametricSpecies::Vertical(VerticalSegmentSpecies::Declared(
                DeclaredKappaRamp::Sine,
            )),
            [0.0, 0.0],
            100.0,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        blend.end_curvature = 0.004;
        // The realized heading of the Sine ramp: theta(t) = 0.4 * W(t/100),
        // W(ksi) = ksi^2/2 + (cos(2*pi*ksi) - 1)/(4*pi^2).
        let law = |t: f64| {
            let xi = t / 100.0;
            0.004 * 100.0 * (xi * xi * 0.5 + ((TWO_PI * xi).cos() - 1.0) / (4.0 * PI * PI))
        };
        let height_at = |u: f64| {
            let steps = 200_000usize;
            let h = u / steps as f64;
            let mut acc = (law(0.0).tan() + law(u).tan()) * 0.5;
            for i in 1..steps {
                acc += law(i as f64 * h).tan();
            }
            acc * h
        };
        for u in [25.0, 50.0, 75.0, 100.0] {
            let sample = sample_chain(&single_chain(ParametricPlane::Profile, &blend), u)
                .unwrap()
                .unwrap();
            let expected = height_at(u);
            assert!(
                (sample.point[1] - expected).abs() < 1e-7,
                "the declared vertical's height at u = {u} drifts from the ramp's own law: {} vs {expected}",
                sample.point[1]
            );
        }
    }

    #[test]
    fn declared_blend_refusals() {
        // The Declared head's validation refusals, each before any change.
        let mut knots = [(0.0, 0.0); 6];
        knots[0] = (0.0, 0.0);
        knots[1] = (0.5, 1.0);
        knots[2] = (1.0, 1.0);
        // The count gate (0 and past-the-table both refuse).
        for count in [0u32, 7u32] {
            let mut seg = plan_piecewise_segment(count, knots);
            seg.start_radius = 0.0;
            let err = audit_chain(&single_chain(ParametricPlane::Plan, &seg)).unwrap_err();
            assert!(
                err.contains("valid knot count"),
                "count {count} refuses honestly: {err}"
            );
        }
        // The knot-table gates: the non-ascending interior knot (with the
        // valid (1, 1) end), the non-zero origin, the unnormalized end,
        // the out-of-range ramp values.
        let mut bad = knots;
        bad[1] = (0.0, 0.5);
        bad[2] = (1.0, 1.0);
        let err = audit_chain(&single_chain(ParametricPlane::Plan, &plan_piecewise_segment(3, bad))).unwrap_err();
        assert!(err.contains("ascend"), "non-ascending knots refuse: {err}");
        let mut bad = knots;
        bad[0] = (0.1, 0.0);
        let err = audit_chain(&single_chain(ParametricPlane::Plan, &plan_piecewise_segment(3, bad))).unwrap_err();
        assert!(err.contains("(0, 0)"), "a non-zero origin refuses: {err}");
        let mut bad = knots;
        bad[2] = (1.0, 0.5);
        let err = audit_chain(&single_chain(ParametricPlane::Plan, &plan_piecewise_segment(3, bad))).unwrap_err();
        assert!(err.contains("(1, 1)"), "an unnormalized end refuses: {err}");
        let mut bad = knots;
        bad[1] = (0.5, 1.5);
        let err = audit_chain(&single_chain(ParametricPlane::Plan, &plan_piecewise_segment(3, bad))).unwrap_err();
        assert!(
            err.contains("in [0, 1]"),
            "an out-of-range ramp value refuses: {err}"
        );
        // The vertical Declared's k1 = 0 refusal (the record-model rule:
        // the flat-out blend restates PARABOLICARC).
        let mut flat_out = profile_segment(
            ParametricSpecies::Vertical(VerticalSegmentSpecies::Declared(
                DeclaredKappaRamp::Linear,
            )),
            [0.0, 0.0],
            100.0,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        flat_out.end_curvature = 0.0;
        let err = audit_chain(&single_chain(ParametricPlane::Profile, &flat_out)).unwrap_err();
        assert!(err.contains("restates PARABOLICARC"), "k1 = 0 refuses: {err}");
    }

    #[test]
    fn declared_blend_seams_with_hardcoded_species() {
        // The audited chain across a Declared/hard-coded seam: the
        // Declared head's end direction feeds the arc's start, propagated
        // from the engine's own sampling; the mixed chain audits green.
        let mut blend = plan_segment(
            ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Declared(
                DeclaredKappaRamp::Cosine,
            )),
            [60.0, 0.0],
            0.0,
            60.0,
            0.0,
            0.0,
        );
        blend.end_curvature = 1.0 / -300.0;
        let front = ParametricChain {
            plane: ParametricPlane::Plan,
            segments: vec![
                plan_segment(
                    ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
                    [0.0, 0.0],
                    0.0,
                    60.0,
                    0.0,
                    0.0,
                ),
                blend,
            ],
            layer: String::new(),
        };
        audit_chain(&front).expect("the front audits");
        let joint = sample_chain(&front, 120.0).unwrap().unwrap();
        let arc = plan_segment(
            ParametricSpecies::Horizontal(HorizontalSegmentSpecies::CircularArc),
            joint.point,
            joint.direction[1].atan2(joint.direction[0]),
            40.0,
            -300.0,
            -300.0,
        );
        let mut chain = front;
        chain.segments.push(arc);
        audit_chain(&chain).expect("the mixed chain audits green");
    }

    /// A one-segment chain around `segment` (the Declared laws' fixture
    /// wrapper), in the segment's plane.
    fn single_chain(plane: ParametricPlane, segment: &ParametricSegment) -> ParametricChain {
        ParametricChain {
            plane,
            segments: vec![*segment],
            layer: String::new(),
        }
    }

    /// The piecewise-knots plan Declared segment for the refusal fixtures.
    fn plan_piecewise_segment(count: u32, knots: [(f64, f64); 6]) -> ParametricSegment {
        let mut segment = plan_segment(
            ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Declared(
                DeclaredKappaRamp::Piecewise { count, knots },
            )),
            [0.0, 0.0],
            0.0,
            60.0,
            0.0,
            0.0,
        );
        segment.end_curvature = 1.0 / 500.0;
        segment
    }

    /// The vitals of a single segment (the parity harness's direct call).
    fn vitals_of(segment: &ParametricSegment) -> Vitals {
        validate_segment(ParametricPlane::Plan, segment).expect("the parity segment validates")
    }
}

/// The v2 generic ramp machinery's own tests (additive now; the switch wave
/// rewires the staging/drag onto these and the named-species arms leave).
#[cfg(test)]
mod generic_ramp_tests {
    use super::*;

    fn ramp_poly(degree: u8, coeffs: [f64; 4]) -> ocs_plugin_api::host::Ramp {
        ocs_plugin_api::host::Ramp::Polynomial { degree, coeffs }
    }

    #[test]
    fn polynomial_ramp_evals_asc_coeffs() {
        // v(xi) = 2 + 3·xi: v(0.5) = 3.5; ∫[0,0.5] = 2·0.5 + 3·0.5²/2 = 1.375.
        let ramp = ramp_poly(1, [2.0, 3.0, 0.0, 0.0]);
        assert!((generic_ramp_eval(&ramp, 0.5) - 3.5).abs() < 1e-12);
        assert!((generic_ramp_integral(&ramp, 0.5) - 1.375).abs() < 1e-12);
        // A constant kappa=1 integrates to the full unit over [0,1].
        let ramp = ramp_poly(0, [1.0, 0.0, 0.0, 0.0]);
        assert!((generic_ramp_integral(&ramp, 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn piecewise_ramp_evals_and_integrates_quadratically() {
        // A triangle: knots (0,0), (0.5,1), (1,0) — value peak 0.5 at xi .25;
        // the exact integral (linear law) = the triangle area = 0.5.
        let ramp = ocs_plugin_api::host::Ramp::Piecewise {
            count: 3,
            knots: [(0.0, 0.0), (0.5, 1.0), (1.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
        };
        assert!((generic_ramp_eval(&ramp, 0.25) - 0.5).abs() < 1e-12);
        assert!((generic_ramp_integral(&ramp, 1.0) - 0.5).abs() < 1e-12);
        assert!((generic_ramp_integral(&ramp, 0.5) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn sampled_ramp_matches_the_piecewise_within_the_grid() {
        // n=201 samples over the same triangle law; the integral within the
        // trapezoid law's local error, the value within one grid step.
        let ramp = ocs_plugin_api::host::Ramp::Sampled {
            n: 201,
            values: (0..201)
                .map(|i| {
                    let xi = i as f64 / 200.0;
                    if xi <= 0.5 {
                        2.0 * xi
                    } else {
                        2.0 - 2.0 * xi
                    }
                })
                .collect(),
        };
        let eval = generic_ramp_eval(&ramp, 0.25);
        assert!((eval - 0.5).abs() < 1.0 / 200.0);
        let total = generic_ramp_integral(&ramp, 1.0);
        assert!(
            (total - 0.5).abs() < 1e-4,
            "sampled trapezoid integral: {total}"
        );
    }

    #[test]
    fn ramp_eval_and_integral_are_bit_deterministic_and_clamped() {
        let ramp = ramp_poly(2, [1.0, -3.0, 2.0, 0.0]);
        let a = generic_ramp_eval(&ramp, 0.3);
        let b = generic_ramp_eval(&ramp, 0.3);
        assert_eq!(a.to_bits(), b.to_bits(), "the eval must be bit-stable");
        let ia = generic_ramp_integral(&ramp, 0.3);
        let ib = generic_ramp_integral(&ramp, 0.3);
        assert_eq!(ia.to_bits(), ib.to_bits(), "the integral must be bit-stable");
        assert!((generic_ramp_eval(&ramp, -2.0) - generic_ramp_eval(&ramp, 0.0)).abs() < 1e-15);
        assert!((generic_ramp_eval(&ramp, 5.0) - generic_ramp_eval(&ramp, 1.0)).abs() < 1e-15);
        assert!((generic_ramp_integral(&ramp, 5.0) - generic_ramp_integral(&ramp, 1.0)).abs() < 1e-15);
    }
}

