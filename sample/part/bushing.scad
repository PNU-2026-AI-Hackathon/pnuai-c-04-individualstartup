// @main_component flanged_bushing

// Nominal dimensions in millimetres
// @param min=20 max=20 step=0.1 label=Inner bore diameter
bore_diameter = 20;
// @param min=30 max=30 step=0.1 label=Body outer diameter
body_diameter = 30;
// @param min=30 max=30 step=0.1 label=Body length
body_length = 30;
// @param min=40 max=40 step=0.1 label=Flange outer diameter
flange_diameter = 40;
// @param min=5 max=5 step=0.1 label=Flange thickness
flange_thickness = 5;
// @param min=0.6 max=1.5 step=0.1 label=Edge chamfer
chamfer = 1;

$fn = 128;
epsilon = 0.05;
overall_length = flange_thickness + body_length;

module flanged_bushing_outer() {
    union() {
        // Flange: full 40 mm OD over its central 3 mm, with 1 mm
        // chamfers on both exposed perimeter edges.
        cylinder(
            h = chamfer,
            d1 = flange_diameter - 2 * chamfer,
            d2 = flange_diameter
        );
        translate([0, 0, chamfer])
            cylinder(
                h = flange_thickness - 2 * chamfer,
                d = flange_diameter
            );
        translate([0, 0, flange_thickness - chamfer])
            cylinder(
                h = chamfer + epsilon,
                d1 = flange_diameter,
                d2 = flange_diameter - 2 * chamfer
            );

        // The 30 mm body begins at the flange's rear face. Its tail edge
        // receives a 1 mm chamfer while the remaining length stays at 30 mm OD.
        translate([0, 0, flange_thickness - epsilon])
            cylinder(
                h = body_length - chamfer + epsilon,
                d = body_diameter
            );
        translate([0, 0, overall_length - chamfer])
            cylinder(
                h = chamfer,
                d1 = body_diameter,
                d2 = body_diameter - 2 * chamfer
            );
    }
}

module shaft_bore() {
    union() {
        // Exact 20 mm through bore.
        translate([0, 0, -epsilon])
            cylinder(h = overall_length + 2 * epsilon, d = bore_diameter);

        // Lead-in chamfer at the flange-side entry.
        translate([0, 0, -epsilon])
            cylinder(
                h = chamfer + epsilon,
                d1 = bore_diameter + 2 * chamfer,
                d2 = bore_diameter
            );

        // Lead-in chamfer at the body-tail entry.
        translate([0, 0, overall_length - chamfer])
            cylinder(
                h = chamfer + epsilon,
                d1 = bore_diameter,
                d2 = bore_diameter + 2 * chamfer
            );
    }
}

difference() {
    flanged_bushing_outer();
    shaft_bore();
}

