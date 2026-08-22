// @main_component Circular Flange Hub

// Circular flange hub dimensions in millimetres.
// @param min=60 max=120 step=1 label=Flange outer diameter
flange_od = 80;
// @param min=6 max=20 step=1 label=Flange thickness
flange_thickness = 10;
// @param min=10 max=40 step=0.5 label=Central bore diameter
bore_diameter = 20;
// @param min=24 max=50 step=1 label=Hub outer diameter
hub_od = 36;
// @param min=10 max=50 step=1 label=Hub projection
hub_projection = 25;
// @param min=40 max=70 step=1 label=Bolt pitch circle diameter
bolt_pcd = 60;
// @param min=4 max=12 step=0.5 label=Bolt hole diameter
bolt_hole_diameter = 8;

bolt_count = 6;
cut_overlap = 0.2;
$fn = 128;

difference() {
    union() {
        // Primary flange plate.
        cylinder(d = flange_od, h = flange_thickness);

        // Integral shaft hub, projecting above the flange.
        translate([0, 0, flange_thickness])
            cylinder(d = hub_od, h = hub_projection);
    }

    // Continuous shaft bore through both flange and hub.
    translate([0, 0, -cut_overlap])
        cylinder(
            d = bore_diameter,
            h = flange_thickness + hub_projection + 2 * cut_overlap
        );

    // Six equally spaced flange bolt holes on the 60 mm PCD.
    for (angle = [0 : 360 / bolt_count : 360 - 360 / bolt_count]) {
        translate([
            (bolt_pcd / 2) * cos(angle),
            (bolt_pcd / 2) * sin(angle),
            -cut_overlap
        ])
            cylinder(
                d = bolt_hole_diameter,
                h = flange_thickness + 2 * cut_overlap,
                $fn = 64
            );
    }
}

