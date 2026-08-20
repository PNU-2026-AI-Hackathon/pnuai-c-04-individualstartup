// @main_component HelicalCompressionSpring

// @param min=2 max=6 step=0.1 label=Wire diameter (mm)
wire_diameter = 3;
// @param min=12 max=60 step=0.5 label=Mean coil diameter (mm)
mean_coil_diameter = 24;
// @param min=3 max=20 step=1 label=Active turns
active_turns = 8;
// @param min=20 max=120 step=1 label=Free length (mm)
free_length = 50;

mean_radius = mean_coil_diameter / 2;
constant_pitch = free_length / active_turns;
total_twist = 360 * active_turns;

// A single twisted extrusion avoids overlapping segment shells. The extrusion
// caps create integral, planar wire ends at z=0 and z=free_length.
slices_per_turn = 48;
profile_facets = 24;

module HelicalCompressionSpring() {
    linear_extrude(
        height = free_length,
        twist = total_twist,
        slices = active_turns * slices_per_turn,
        convexity = 20
    )
        translate([mean_radius, 0])
            circle(d = wire_diameter, $fn = profile_facets);
}

HelicalCompressionSpring();

