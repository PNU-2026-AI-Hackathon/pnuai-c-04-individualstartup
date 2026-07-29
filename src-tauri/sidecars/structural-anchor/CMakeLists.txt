cmake_minimum_required(VERSION 3.20)

project(cadastrophe_structural_anchor LANGUAGES CXX)

option(
  CADASTROPHE_STRUCTURAL_ANCHOR_USE_OPEN3D
  "Use Open3D when find_package(Open3D) succeeds."
  ON
)
option(
  CADASTROPHE_STRUCTURAL_ANCHOR_REQUIRE_OPEN3D
  "Fail configuration when Open3D is not available."
  OFF
)

add_executable(cadastrophe-structural-anchor src/main.cpp)
target_compile_features(cadastrophe-structural-anchor PRIVATE cxx_std_17)

if(CADASTROPHE_STRUCTURAL_ANCHOR_USE_OPEN3D)
  find_package(Open3D QUIET)
  if(Open3D_FOUND)
    target_compile_definitions(cadastrophe-structural-anchor PRIVATE CADASTROPHE_WITH_OPEN3D=1)
    target_link_libraries(cadastrophe-structural-anchor PRIVATE Open3D::Open3D)
  elseif(CADASTROPHE_STRUCTURAL_ANCHOR_REQUIRE_OPEN3D)
    message(FATAL_ERROR "Open3D was required but find_package(Open3D) failed.")
  else()
    message(STATUS "Open3D not found; building deterministic fallback STL evaluator.")
  endif()
else()
  message(STATUS "Open3D disabled; building deterministic fallback STL evaluator.")
endif()
