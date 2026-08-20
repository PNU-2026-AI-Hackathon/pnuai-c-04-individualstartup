// @main_component driving_spur_gear
// Meshing involute spur gear pair for FFF additive manufacturing.
// Pitch diameters: 40 mm / 80 mm. Exact center distance: 60 mm.
// Speed ratio: 2:1; external mesh produces opposite rotation directions.

$fn = 128;

// @param min=1 max=4 step=0.25 label=Module
gear_module = 2;
// @param min=14.5 max=25 step=0.5 label=Pressure angle
pressure_angle = 20;
// @param min=6 max=20 step=1 label=Face width
face_width = 10;
// Total mesh backlash is 0.20 mm because each gear tooth is narrowed by 0.10 mm.
tooth_thickness_reduction = 0.10;
involute_steps = 7;

function rot2(p, a) = [
    p[0] * cos(a) - p[1] * sin(a),
    p[0] * sin(a) + p[1] * cos(a)
];

// Involute parameter t is supplied in degrees for OpenSCAD trigonometry.
function involute_xy(base_r, t) = [
    base_r * (cos(t) + (t * PI / 180) * sin(t)),
    base_r * (sin(t) - (t * PI / 180) * cos(t))
];

function involute_polar_angle(t) = t - atan(t * PI / 180);

// One symmetric tooth. Its upper flank is a reflected involute so the tooth
// narrows correctly from base circle to addendum circle.
module involute_tooth(teeth, module_size, pa, thickness_reduction) {
    pitch_r = module_size * teeth / 2;
    base_r = pitch_r * cos(pa);
    outer_r = pitch_r + module_size;
    root_r = pitch_r - 1.25 * module_size;
    pitch_inv_angle = tan(pa) * 180 / PI - pa;
    half_tooth_angle = ((PI * module_size / 2 - thickness_reduction) / pitch_r) * 90 / PI;
    base_half_angle = half_tooth_angle + pitch_inv_angle;
    outer_t = sqrt((outer_r / base_r) * (outer_r / base_r) - 1) * 180 / PI;

    upper = [
        for (i = [0 : involute_steps])
            let(t = outer_t * i / involute_steps,
                q = involute_xy(base_r, t))
            rot2([q[0], -q[1]], base_half_angle)
    ];
    lower = [for (i = [0 : involute_steps]) [upper[i][0], -upper[i][1]]];

    polygon(points = concat(
        [[root_r * cos(base_half_angle), -root_r * sin(base_half_angle)]],
        lower,
        [for (i = [involute_steps : -1 : 0]) upper[i]],
        [[root_r * cos(base_half_angle), root_r * sin(base_half_angle)]]
    ));
}

module spur_gear(teeth, bore_d, module_size=2, pa=20, width=10, phase=0) {
    pitch_r = module_size * teeth / 2;
    root_r = pitch_r - 1.25 * module_size;

    difference() {
        linear_extrude(height = width, convexity = 12)
            rotate(phase)
                union() {
                    circle(r = root_r, $fn = max(96, teeth * 4));
                    for (tooth = [0 : teeth - 1])
                        rotate(tooth * 360 / teeth)
                            involute_tooth(teeth, module_size, pa, tooth_thickness_reduction);
                }
        translate([0, 0, -1])
            cylinder(d = bore_d, h = width + 2, $fn = 64);
    }
}

// Driving gear: 20 teeth, pitch diameter 40 mm, 8 mm bore.
spur_gear(
    teeth = 20,
    bore_d = 8,
    module_size = gear_module,
    pa = pressure_angle,
    width = face_width,
    phase = 0
);

// Driven gear: 40 teeth, pitch diameter 80 mm, 10 mm bore.
// A half-tooth phase places a gap on the line of centers for correct initial mesh.
translate([60, 0, 0])
    spur_gear(
        teeth = 40,
        bore_d = 10,
        module_size = gear_module,
        pa = pressure_angle,
        width = face_width,
        phase = 180 / 40
    );

