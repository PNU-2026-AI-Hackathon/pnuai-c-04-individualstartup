// @main_component SingleGrooveVBeltPulley

// Single-groove V-belt pulley for FFF manufacturing.
// Overall envelope: 80 mm diameter x 20 mm width.

// @param min=60 max=120 step=1 label=Pulley outer diameter (mm)
outer_diameter = 80;
// @param min=16 max=30 step=1 label=Pulley width (mm)
pulley_width = 20;
// @param min=8 max=25 step=0.5 label=Shaft bore diameter (mm)
bore_diameter = 15;
// @param min=32 max=42 step=1 label=Groove included angle (degrees)
groove_angle = 38;
// @param min=5 max=10 step=0.5 label=Groove radial depth (mm)
groove_depth = 8;
// @param min=1.2 max=3 step=0.2 label=Groove root land (mm)
groove_root_width = 1.6;
// @param min=30 max=44 step=1 label=Hub diameter (mm)
hub_diameter = 36;

rim_inner_radius = 29;
web_radius = 30.5;
web_thickness = 8;
edge_chamfer = 0.8;
epsilon = 0.2;

outer_radius = outer_diameter / 2;
groove_root_radius = outer_radius - groove_depth;
groove_root_half_width = groove_root_width / 2;
groove_cutter_outer_radius = outer_radius + 0.5;
groove_cutter_half_width = groove_root_half_width
    + (groove_cutter_outer_radius - groove_root_radius)
    * tan(groove_angle / 2);

$fn = 160;

module chamfered_rim() {
    rotate_extrude(convexity = 10)
        polygon(points = [
            [rim_inner_radius, 0],
            [outer_radius - edge_chamfer, 0],
            [outer_radius, edge_chamfer],
            [outer_radius, pulley_width - edge_chamfer],
            [outer_radius - edge_chamfer, pulley_width],
            [rim_inner_radius, pulley_width]
        ]);
}

module integrated_blank() {
    union() {
        chamfered_rim();

        // Recessed torque-carrying web, overlapping the rim radially.
        translate([0, 0, (pulley_width - web_thickness) / 2])
            cylinder(h = web_thickness, r = web_radius);

        // Full-width shaft hub.
        cylinder(h = pulley_width, d = hub_diameter);
    }
}

module centered_v_groove_cutter() {
    translate([0, 0, pulley_width / 2])
        rotate_extrude(convexity = 10)
            polygon(points = [
                [groove_root_radius, -groove_root_half_width],
                [groove_cutter_outer_radius, -groove_cutter_half_width],
                [groove_cutter_outer_radius, groove_cutter_half_width],
                [groove_root_radius, groove_root_half_width]
            ]);
}

difference() {
    integrated_blank();
    centered_v_groove_cutter();

    // Exact centered 15 mm shaft bore, extended slightly for a clean through-cut.
    translate([0, 0, -epsilon])
        cylinder(h = pulley_width + 2 * epsilon, d = bore_diameter);
}

