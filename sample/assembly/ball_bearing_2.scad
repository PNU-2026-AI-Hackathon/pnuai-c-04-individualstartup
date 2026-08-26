// @main_component Outer Ring
// Topology-safe, visibly open, print-in-place deep-groove bearing: 20 x 47 x 14 mm.
// Independent solids: 1 outer ring + 1 inner ring + 8 balls + 1 cage.

$fa = 3;
$fs = 0.35;

// @param min=46 max=48 step=0.1 label=Outer diameter (mm)
outer_diameter = 47;
// @param min=19 max=21 step=0.1 label=Bore diameter (mm)
bore_diameter = 20;
// @param min=13 max=15 step=0.1 label=Overall width (mm)
bearing_width = 14;
// @param min=5.6 max=6.2 step=0.05 label=Ball diameter (mm)
ball_diameter = 6.0;

pitch_radius = 16.75;
ball_radius = ball_diameter / 2;

// Each ring is generated from one simple closed cross-section.  The sampled
// circular arcs are the 3.30 mm raceway radius: 3.00 mm ball + 0.30 mm gap.
module outer_ring() {
    rotate_extrude(convexity=10, $fn=160)
        polygon(points=[
            [23.500, -7.000],
            [23.500,  7.000],
            [20.250,  7.000],
            [20.250,  2.700],
            [18.647,  2.700],
            [19.015,  2.400],
            [19.296,  2.100],
            [19.516,  1.800],
            [19.689,  1.500],
            [19.824,  1.200],
            [19.925,  0.900],
            [19.995,  0.600],
            [20.036,  0.300],
            [20.050,  0.000],
            [20.036, -0.300],
            [19.995, -0.600],
            [19.925, -0.900],
            [19.824, -1.200],
            [19.689, -1.500],
            [19.516, -1.800],
            [19.296, -2.100],
            [19.015, -2.400],
            [18.647, -2.700],
            [20.250, -2.700],
            [20.250, -7.000]
        ]);
}

module inner_ring() {
    rotate_extrude(convexity=10, $fn=160)
        polygon(points=[
            [10.000, -7.000],
            [13.250, -7.000],
            [13.250, -2.700],
            [14.853, -2.700],
            [14.485, -2.400],
            [14.204, -2.100],
            [13.984, -1.800],
            [13.811, -1.500],
            [13.676, -1.200],
            [13.575, -0.900],
            [13.505, -0.600],
            [13.464, -0.300],
            [13.450,  0.000],
            [13.464,  0.300],
            [13.505,  0.600],
            [13.575,  0.900],
            [13.676,  1.200],
            [13.811,  1.500],
            [13.984,  1.800],
            [14.204,  2.100],
            [14.485,  2.400],
            [14.853,  2.700],
            [13.250,  2.700],
            [13.250,  7.000],
            [10.000,  7.000]
        ]);
}

module rolling_balls() {
    for (angle = [0:45:315])
        rotate([0, 0, angle])
            translate([pitch_radius, 0, 0])
                sphere(r=ball_radius, $fn=56);
}

module cage_rear_rail() {
    // One rectangular profile produces a watertight annular rail directly.
    rotate_extrude(convexity=6, $fn=144)
        polygon(points=[
            [14.40, -4.30],
            [19.10, -4.30],
            [19.10, -3.30],
            [14.40, -3.30]
        ]);
}

module cage() {
    union() {
        cage_rear_rail();

        // Eight robust posts overlap the rail and sit halfway between balls.
        for (angle = [22.5:45:337.5])
            rotate([0, 0, angle])
                translate([pitch_radius, 0, -1.00])
                    cylinder(h=5.60, r=0.80, center=true, $fn=36);
    }
}

color([0.72, 0.75, 0.78]) outer_ring();
color([0.58, 0.62, 0.66]) inner_ring();
color([0.94, 0.95, 0.97]) rolling_balls();
color([0.95, 0.55, 0.10]) cage();

