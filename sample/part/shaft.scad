// @main_component stepped_shaft
// Functional stepped shaft for FFF additive manufacturing.
// Axis is Z; all dimensions are millimetres.

// @param min=80 max=140 step=5 label=Overall length (mm)
overall_length = 100;
// @param min=16 max=25 step=1 label=Journal diameter (mm)
journal_diameter = 20;
// @param min=25 max=40 step=1 label=Center diameter (mm)
center_diameter = 30;
// @param min=30 max=60 step=1 label=Center length (mm)
center_length = 40;
// @param min=1 max=3 step=0.25 label=Shoulder fillet radius (mm)
fillet_radius = 2;
// @param min=4 max=8 step=1 label=Keyway width (mm)
keyway_width = 6;
// @param min=2 max=5 step=0.5 label=Keyway depth (mm)
keyway_depth = 3.5;
// @param min=15 max=28 step=1 label=Keyway length (mm)
keyway_length = 25;

$fn = 160;

// Revolved axial profile. The sampled circular segments are true R2
// quarter-circle shoulder fillets, not straight chamfers.
module shaft_blank() {
    rotate_extrude(angle = 360, convexity = 10)
        polygon(points = [
            [0, 0],
            [9.2, 0],
            [10, 0.8],
            [10, 28],
            [10.017, 28.261],
            [10.068, 28.518],
            [10.152, 28.765],
            [10.268, 29.000],
            [10.414, 29.218],
            [10.586, 29.414],
            [10.782, 29.586],
            [11.000, 29.732],
            [11.235, 29.848],
            [11.482, 29.932],
            [11.739, 29.983],
            [12, 30],
            [15, 30],
            [15, 70],
            [12, 70],
            [11.739, 70.017],
            [11.482, 70.068],
            [11.235, 70.152],
            [11.000, 70.268],
            [10.782, 70.414],
            [10.586, 70.586],
            [10.414, 70.782],
            [10.268, 71.000],
            [10.152, 71.235],
            [10.068, 71.482],
            [10.017, 71.739],
            [10, 72],
            [10, 99.2],
            [9.2, 100],
            [0, 100]
        ]);
}

// Open-ended 6 x 3.5 mm keyway. The transverse cylinder represents
// a 6 mm end mill and gives the blind end a radius of 3 mm.
module keyway_cut() {
    union() {
        translate([-keyway_width / 2, journal_diameter / 2 - keyway_depth, -0.1])
            cube([keyway_width, keyway_depth + 1.0, keyway_length - keyway_width / 2 + 0.1]);

        translate([0, journal_diameter / 2 - keyway_depth, keyway_length - keyway_width / 2])
            rotate([-90, 0, 0])
                cylinder(h = keyway_depth + 1.0, r = keyway_width / 2, $fn = 64);
    }
}

module stepped_shaft() {
    difference() {
        shaft_blank();
        keyway_cut();
    }
}

stepped_shaft();

