cmake_minimum_required(VERSION 3.20)
project(cadastrophe_mesh_validator_parity LANGUAGES CXX)

find_package(Open3D CONFIG REQUIRED)

add_executable(mesh-validator-parity
    parity.cpp
    ../../src-tauri/sidecars/structural-anchor/src/mesh_validator.cpp)
target_compile_features(mesh-validator-parity PRIVATE cxx_std_17)
target_include_directories(mesh-validator-parity PRIVATE
    ../../src-tauri/sidecars/structural-anchor/src)
target_link_libraries(mesh-validator-parity PRIVATE Open3D::Open3D)

enable_testing()
add_test(NAME mesh-validator-parity COMMAND mesh-validator-parity)
