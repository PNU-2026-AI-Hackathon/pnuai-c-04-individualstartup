// @main_component radial_disk_cam

// Radial disk cam for a translating knife-edge follower.
// Displacement law over one revolution:
//   lift(theta) = lift_max/2 * (1 - cos(theta))
// This gives zero lift at 0/360 degrees, maximum lift at 180 degrees,
// and zero profile slope at all three transition positions.

// @param min=25 max=25 step=1 label=Base circle radius (mm)
base_radius = 25;

// @param min=15 max=15 step=1 label=Maximum lift (mm)
lift_max = 15;

// @param min=10 max=10 step=1 label=Cam thickness (mm)
cam_thickness = 10;

// @param min=10 max=10 step=1 label=Center bore diameter (mm)
bore_diameter = 10;

// One-degree sampling keeps polygon facets well below the 0.4 mm nozzle scale.
profile_segments = 360;
bore_segments = 96;

profile_points = [
    for (i = [0 : profile_segments - 1])
        let(
            angle = i * 360 / profile_segments,
            follower_lift = lift_max / 2 * (1 - cos(angle)),
            profile_radius = base_radius + follower_lift
        )
        [
            profile_radius * cos(angle),
            profile_radius * sin(angle)
        ]
];

difference() {
    linear_extrude(height = cam_thickness, convexity = 10)
        polygon(points = profile_points);

    // Extend the cutter beyond both faces to guarantee a clean through bore.
    translate([0, 0, -0.5])
        cylinder(
            h = cam_thickness + 1,
            d = bore_diameter,
            $fn = bore_segments
        );
}

