// @main_component M10_hex_head_bolt
// Regular manifold CSG M10-profile partial-thread hex-head bolt; millimetres.

// @param min=8 max=12 step=1 label=Nominal thread diameter (mm)
nominal_diameter = 10;
// @param min=1 max=2 step=0.25 label=Thread pitch (mm)
thread_pitch = 1.5;
// @param min=40 max=70 step=1 label=Overall length (mm)
overall_length = 50;

head_height = 6.4;
head_across_flats = 16;
head_corner_radius = head_across_flats/(2*cos(30));
thread_length = 26;
thread_start = overall_length-thread_length;
major_radius = nominal_diameter/2;
minor_radius = 8.16/2;
tip_chamfer = 0.75;
thread_profile_end = overall_length-tip_chamfer;

crest_half_fraction = 0.10;
root_start_fraction = 0.45;
profile_samples = ceil((thread_profile_end-thread_start)/(thread_pitch/12));

function frac(value) = value-floor(value);
function axial_thread_distance(z) =
    abs(frac((z-thread_start)/thread_pitch+0.5)-0.5);
function axial_thread_radius(z) =
    let(distance=axial_thread_distance(z))
    distance <= crest_half_fraction ? major_radius :
    distance >= root_start_fraction ? minor_radius :
    major_radius-
        (major_radius-minor_radius)*
        (distance-crest_half_fraction)/
        (root_start_fraction-crest_half_fraction);

thread_profile_points = [
    for (sample=[0:profile_samples])
        let(z=thread_start+
            (thread_profile_end-thread_start)*sample/profile_samples)
        [axial_thread_radius(z),z]
];

// One simple, closed axial polygon produces the complete shoulder, core,
// repeating M10x1.5 60-degree profile, and tapered lead-in as one solid.
shaft_profile = concat(
    [
        [0,6.12],
        [major_radius+0.50,6.12],
        [major_radius+0.50,6.30],
        [major_radius,6.82],
        [major_radius,thread_start-0.10]
    ],
    thread_profile_points,
    [
        [4.62,49.52],
        [minor_radius-0.10,overall_length],
        [0,overall_length]
    ]
);

module closed_rotational_threaded_shaft() {
    rotate_extrude($fn=96, convexity=12)
        polygon(points=shaft_profile);
}

module chamfered_hex_head() {
    union() {
        // Crown edge chamfer.
        cylinder(h=0.82, r1=head_corner_radius-0.72,
                 r2=head_corner_radius, $fn=6);

        translate([0,0,0.80])
            cylinder(h=head_height-1.58,
                     r=head_corner_radius, $fn=6);

        // Small underside edge break while retaining the bearing face.
        translate([0,0,head_height-0.80])
            cylinder(h=0.80, r1=head_corner_radius,
                     r2=head_corner_radius-0.28, $fn=6);
    }
}

// Standard CSG Boolean evaluation removes the head/neck overlap before export.
render(convexity=12)
    union() {
        chamfered_hex_head();
        closed_rotational_threaded_shaft();
    }

