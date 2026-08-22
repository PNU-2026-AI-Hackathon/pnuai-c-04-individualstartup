// @main_component Geneva wheel
// Refined six-slot external Geneva mechanism, units: mm

$fn = 96;

// @param min=50 max=80 step=1 label=Center distance
center_distance = 60;
// @param min=25 max=40 step=0.5 label=Drive pin orbit radius
pin_orbit_radius = 30;
// @param min=4 max=8 step=0.2 label=Working drive pin diameter
pin_diameter = 6;
// @param min=6.4 max=8 step=0.2 label=Slot width
slot_width = 6.8;
// @param min=7 max=10 step=0.2 label=Shaft bore diameter
bore_diameter = 8;

geneva_radius = 54;
geneva_thickness = 6;
geneva_z = 4.2;
slot_inner_radius = 24.5;
slot_outer_radius = 58;

driver_hub_radius = 22;
driver_body_thickness = 3.6;
crank_arm_width = 10;
driver_angle = -40;
geneva_phase = 27.5;

pin_base_z = 3.4;
geneva_top_z = geneva_z + geneva_thickness;
pin_top_z = geneva_top_z + 5;
pin_height = pin_top_z - pin_base_z;

locking_cam_radius = 24;
locking_cam_height = 4;
locking_clearance = 0.5;

// Pin position for the shown partway-through-index engagement pose.
pin_x = pin_orbit_radius * cos(driver_angle);
pin_y = pin_orbit_radius * sin(driver_angle);

// Open-ended radial slot with a round inner end.
module radial_slot_cut(slot_angle) {
    translate([center_distance, 0, 0])
        rotate([0, 0, slot_angle])
            union() {
                translate([slot_inner_radius, -slot_width/2, geneva_z - 0.2])
                    cube([slot_outer_radius - slot_inner_radius,
                          slot_width,
                          geneva_thickness + 0.4]);
                translate([slot_inner_radius, 0, geneva_z - 0.2])
                    cylinder(h = geneva_thickness + 0.4,
                             d = slot_width);
            }
}

// Concave dwell relief between neighboring slots.
module locking_relief_cut(relief_angle) {
    translate([center_distance, 0, 0])
        rotate([0, 0, relief_angle])
            translate([center_distance, 0, geneva_z - 0.2])
                cylinder(h = geneva_thickness + 0.4, r = 28.5);
}

module geneva_wheel() {
    difference() {
        translate([center_distance, 0, geneva_z])
            cylinder(h = geneva_thickness, r = geneva_radius);

        // Exactly six slots at 60 degree spacing.  The phase places the
        // lower-left active slot directly around the offset drive pin.
        for (a = [0 : 60 : 300])
            radial_slot_cut(geneva_phase + a);

        for (a = [30 : 60 : 330])
            locking_relief_cut(geneva_phase + a);

        // 8 mm driven-axis bore.
        translate([center_distance, 0, geneva_z - 0.2])
            cylinder(h = geneva_thickness + 0.4, d = bore_diameter);
    }
}

module crank_arm() {
    // Ten-millimeter-wide arm visibly joins the round driving hub to the pin.
    rotate([0, 0, driver_angle])
        translate([0, -crank_arm_width/2, 0])
            cube([pin_orbit_radius, crank_arm_width, driver_body_thickness]);
}

module driver_locking_cam() {
    // Integral raised crescent with a 0.5 mm relief from the Geneva body.
    difference() {
        translate([0, 0, pin_base_z])
            cylinder(h = locking_cam_height, r = locking_cam_radius);
        translate([center_distance, 0, pin_base_z - 0.2])
            cylinder(h = locking_cam_height + 0.4,
                     r = geneva_radius + locking_clearance);
    }
}

module driving_wheel_with_pin() {
    difference() {
        union() {
            // Circular driver hub plus one radial crank arm form one component.
            cylinder(h = driver_body_thickness, r = driver_hub_radius);
            crank_arm();
            driver_locking_cam();

            // Exactly one solid Ø6 mm working pin.  It passes through the
            // active 6.8 mm slot and rises 5 mm above the Geneva top face.
            translate([pin_x, pin_y, pin_base_z])
                cylinder(h = pin_height, d = pin_diameter);
        }

        // 8 mm driving-axis bore through all coaxial driver geometry.
        translate([0, 0, -0.2])
            cylinder(h = pin_top_z + 0.4, d = bore_diameter);
    }
}

// Independent solids in a functional, visibly engaged assembly pose.
geneva_wheel();
driving_wheel_with_pin();

