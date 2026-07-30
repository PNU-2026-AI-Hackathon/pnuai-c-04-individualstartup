cmake_minimum_required(VERSION 3.20)

project(cadastrophe_structural_anchor LANGUAGES CXX)

add_executable(cadastrophe-structural-anchor src/main.cpp)
target_compile_features(cadastrophe-structural-anchor PRIVATE cxx_std_17)
