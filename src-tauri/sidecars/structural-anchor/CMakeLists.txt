cmake_minimum_required(VERSION 3.20)

project(cadastrophe_structural_anchor LANGUAGES CXX)

add_library(cadastrophe-mesh-validator STATIC
    src/mesh_validator.cpp
    src/mesh_intersection_fixed.cpp)
target_compile_features(cadastrophe-mesh-validator PUBLIC cxx_std_17)
target_include_directories(cadastrophe-mesh-validator PUBLIC src)

add_executable(cadastrophe-structural-anchor src/main.cpp)
target_compile_features(cadastrophe-structural-anchor PRIVATE cxx_std_17)
target_link_libraries(cadastrophe-structural-anchor PRIVATE cadastrophe-mesh-validator)

include(CTest)
if(BUILD_TESTING)
    add_executable(cadastrophe-mesh-validator-test tests/mesh_validator_test.cpp)
    target_compile_features(cadastrophe-mesh-validator-test PRIVATE cxx_std_17)
    target_link_libraries(cadastrophe-mesh-validator-test PRIVATE cadastrophe-mesh-validator)
    add_test(NAME cadastrophe-mesh-validator-test COMMAND cadastrophe-mesh-validator-test)

    add_executable(cadastrophe-mesh-intersection-fixed-test
        tests/mesh_intersection_fixed_test.cpp)
    target_compile_features(cadastrophe-mesh-intersection-fixed-test PRIVATE cxx_std_17)
    target_link_libraries(cadastrophe-mesh-intersection-fixed-test PRIVATE
        cadastrophe-mesh-validator)
    add_test(NAME cadastrophe-mesh-intersection-fixed-test
        COMMAND cadastrophe-mesh-intersection-fixed-test)
endif()
