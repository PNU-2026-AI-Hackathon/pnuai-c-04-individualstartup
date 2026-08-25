cmake_minimum_required(VERSION 3.20)

project(cadgen_ax_structural_anchor LANGUAGES CXX)

add_library(cadgen-ax-mesh-validator STATIC
    src/mesh_validator.cpp
    src/mesh_intersection_fixed.cpp)
target_compile_features(cadgen-ax-mesh-validator PUBLIC cxx_std_17)
target_include_directories(cadgen-ax-mesh-validator PUBLIC src)

add_executable(cadgen-ax-structural-anchor src/main.cpp)
target_compile_features(cadgen-ax-structural-anchor PRIVATE cxx_std_17)
target_link_libraries(cadgen-ax-structural-anchor PRIVATE cadgen-ax-mesh-validator)

include(CTest)
if(BUILD_TESTING)
    add_executable(cadgen-ax-mesh-validator-test tests/mesh_validator_test.cpp)
    target_compile_features(cadgen-ax-mesh-validator-test PRIVATE cxx_std_17)
    target_link_libraries(cadgen-ax-mesh-validator-test PRIVATE cadgen-ax-mesh-validator)
    add_test(NAME cadgen-ax-mesh-validator-test COMMAND cadgen-ax-mesh-validator-test)

    add_executable(cadgen-ax-mesh-intersection-fixed-test
        tests/mesh_intersection_fixed_test.cpp)
    target_compile_features(cadgen-ax-mesh-intersection-fixed-test PRIVATE cxx_std_17)
    target_link_libraries(cadgen-ax-mesh-intersection-fixed-test PRIVATE
        cadgen-ax-mesh-validator)
    add_test(NAME cadgen-ax-mesh-intersection-fixed-test
        COMMAND cadgen-ax-mesh-intersection-fixed-test)
endif()
