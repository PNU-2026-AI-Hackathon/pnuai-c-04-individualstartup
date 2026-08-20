// @main_component 24T_Involute_Spur_Gear

// Watertight functional spur gear: one continuous outer polygon is extruded,
// then one concentric bore is subtracted. No tooth/root solids overlap.
// Standard full-depth metric geometry:
//   z = 24, m = 2 mm, alpha = 20 degrees
//   pitch diameter = 48 mm
//   outside diameter = 52 mm
//   root diameter = 43 mm
//   face width = 10 mm, bore diameter = 8 mm

$fn = 128;

// @param min=12 max=60 step=1 label=Tooth count
teeth = 24;
// @param min=1 max=4 step=0.25 label=Module (mm)
module_size = 2;
// @param min=14.5 max=25 step=0.5 label=Pressure angle (deg)
pressure_angle = 20;
// @param min=4 max=20 step=1 label=Face width (mm)
face_width = 10;
// @param min=3 max=16 step=0.5 label=Bore diameter (mm)
bore_diameter = 8;

pitch_radius = module_size * teeth / 2;
outside_radius = pitch_radius + module_size;
root_radius = pitch_radius - 1.25 * module_size;
base_radius = pitch_radius * cos(pressure_angle);

tooth_pitch_angle = 360 / teeth;
sector_half_angle = tooth_pitch_angle / 2;
pitch_half_thickness_angle = 90 / teeth;

function involute_parameter(radius) =
    sqrt((radius * radius) / (base_radius * base_radius) - 1);

function involute_roll_degrees(t) =
    t * 180 / PI - atan(t);

function polar_point(radius, angle) =
    [radius * cos(angle), radius * sin(angle)];

function flank_radius(t) =
    base_radius * sqrt(1 + t * t);

pitch_t = involute_parameter(pitch_radius);
outside_t = involute_parameter(outside_radius);
base_half_angle =
    pitch_half_thickness_angle + involute_roll_degrees(pitch_t);
tip_half_angle =
    base_half_angle - involute_roll_degrees(outside_t);

// A compact under-base transition preserves standard tip clearance while
// joining the true involute smoothly into the dedendum region.
root_half_angle = base_half_angle + 1.0;

flank_steps = 16;
tip_arc_steps = 6;
root_arc_steps = 4;

// One complete tooth sector, ordered strictly counter-clockwise.
// The final sector-boundary point is deliberately omitted; it is supplied
// once by the beginning of the following sector, preventing zero-length edges.
function tooth_sector_points(center_angle) = concat(
    // Root arc approaching the negative flank transition.
    [for (i = [0 : root_arc_steps])
        polar_point(
            root_radius,
            center_angle - sector_half_angle
                + (sector_half_angle - root_half_angle) * i / root_arc_steps
        )
    ],

    // Negative involute flank, base circle to outside circle.
    [for (i = [0 : flank_steps])
        let(t = outside_t * i / flank_steps)
        polar_point(
            flank_radius(t),
            center_angle - base_half_angle + involute_roll_degrees(t)
        )
    ],

    // Tooth-tip arc; its first point is omitted to avoid duplicating the flank tip.
    [for (i = [1 : tip_arc_steps])
        polar_point(
            outside_radius,
            center_angle - tip_half_angle
                + 2 * tip_half_angle * i / tip_arc_steps
        )
    ],

    // Positive involute flank, outside circle back to base circle.
    [for (i = [flank_steps - 1 : -1 : 0])
        let(t = outside_t * i / flank_steps)
        polar_point(
            flank_radius(t),
            center_angle + base_half_angle - involute_roll_degrees(t)
        )
    ],

    // Positive flank transition into the root.
    [polar_point(root_radius, center_angle + root_half_angle)],

    // Root arc toward the next tooth; endpoint omitted for unique vertices.
    [for (i = [1 : root_arc_steps - 1])
        polar_point(
            root_radius,
            center_angle + root_half_angle
                + (sector_half_angle - root_half_angle) * i / root_arc_steps
        )
    ]
);

function gear_perimeter_points() = [
    for (tooth_index = [0 : teeth - 1])
        each tooth_sector_points(tooth_index * tooth_pitch_angle)
];

module gear_outline_2d() {
    polygon(points = gear_perimeter_points(), convexity = 10);
}

module finished_spur_gear() {
    difference() {
        linear_extrude(height = face_width, convexity = 10)
            gear_outline_2d();

        translate([0, 0, -0.5])
            cylinder(
                h = face_width + 1,
                d = bore_diameter,
                $fn = 96
            );
    }
}

finished_spur_gear();

