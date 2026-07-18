cmake_minimum_required(VERSION 3.20)

project(badness_cst_ffi_example LANGUAGES CXX)

include(CTest)

set(BADNESS_ROOT "${CMAKE_CURRENT_LIST_DIR}/../..")
set(BADNESS_DEBUG_DIR "${BADNESS_ROOT}/target/debug")
set(BADNESS_RELEASE_DIR "${BADNESS_ROOT}/target/release")
set(BADNESS_RELEASE_CONFIG
    "$<OR:$<CONFIG:Release>,$<CONFIG:RelWithDebInfo>,$<CONFIG:MinSizeRel>>"
)

# CMake owns the C++ compile/link step; Cargo builds the cdylib before the linker
# consumes it. Keeping Rust as a custom target avoids a second, CMake-specific Rust
# build configuration.
add_custom_target(badness_rust_ffi
    COMMAND cargo build --manifest-path "${BADNESS_ROOT}/Cargo.toml"
            "$<${BADNESS_RELEASE_CONFIG}:--release>"
    WORKING_DIRECTORY "${BADNESS_ROOT}"
    COMMENT "Building the badness Rust cdylib"
)

if(WIN32)
    set(BADNESS_DEBUG_LIBRARY "${BADNESS_DEBUG_DIR}/badness.dll")
    set(BADNESS_RELEASE_LIBRARY "${BADNESS_RELEASE_DIR}/badness.dll")
    set(BADNESS_DEBUG_IMPLIB "${BADNESS_DEBUG_DIR}/badness.dll.lib")
    set(BADNESS_RELEASE_IMPLIB "${BADNESS_RELEASE_DIR}/badness.dll.lib")
    add_library(badness_ffi SHARED IMPORTED GLOBAL)
    set_target_properties(badness_ffi PROPERTIES
        IMPORTED_LOCATION "${BADNESS_DEBUG_LIBRARY}"
        IMPORTED_LOCATION_DEBUG "${BADNESS_DEBUG_LIBRARY}"
        IMPORTED_LOCATION_RELEASE "${BADNESS_RELEASE_LIBRARY}"
        IMPORTED_LOCATION_RELWITHDEBINFO "${BADNESS_RELEASE_LIBRARY}"
        IMPORTED_LOCATION_MINSIZEREL "${BADNESS_RELEASE_LIBRARY}"
        IMPORTED_IMPLIB "${BADNESS_DEBUG_IMPLIB}"
        IMPORTED_IMPLIB_DEBUG "${BADNESS_DEBUG_IMPLIB}"
        IMPORTED_IMPLIB_RELEASE "${BADNESS_RELEASE_IMPLIB}"
        IMPORTED_IMPLIB_RELWITHDEBINFO "${BADNESS_RELEASE_IMPLIB}"
        IMPORTED_IMPLIB_MINSIZEREL "${BADNESS_RELEASE_IMPLIB}"
    )
elseif(APPLE)
    set(BADNESS_DEBUG_LIBRARY "${BADNESS_DEBUG_DIR}/libbadness.dylib")
    set(BADNESS_RELEASE_LIBRARY "${BADNESS_RELEASE_DIR}/libbadness.dylib")
    add_library(badness_ffi SHARED IMPORTED GLOBAL)
    set_target_properties(badness_ffi PROPERTIES
        IMPORTED_LOCATION "${BADNESS_DEBUG_LIBRARY}"
        IMPORTED_LOCATION_DEBUG "${BADNESS_DEBUG_LIBRARY}"
        IMPORTED_LOCATION_RELEASE "${BADNESS_RELEASE_LIBRARY}"
        IMPORTED_LOCATION_RELWITHDEBINFO "${BADNESS_RELEASE_LIBRARY}"
        IMPORTED_LOCATION_MINSIZEREL "${BADNESS_RELEASE_LIBRARY}"
    )
else()
    set(BADNESS_DEBUG_LIBRARY "${BADNESS_DEBUG_DIR}/libbadness.so")
    set(BADNESS_RELEASE_LIBRARY "${BADNESS_RELEASE_DIR}/libbadness.so")
    add_library(badness_ffi SHARED IMPORTED GLOBAL)
    set_target_properties(badness_ffi PROPERTIES
        IMPORTED_LOCATION "${BADNESS_DEBUG_LIBRARY}"
        IMPORTED_LOCATION_DEBUG "${BADNESS_DEBUG_LIBRARY}"
        IMPORTED_LOCATION_RELEASE "${BADNESS_RELEASE_LIBRARY}"
        IMPORTED_LOCATION_RELWITHDEBINFO "${BADNESS_RELEASE_LIBRARY}"
        IMPORTED_LOCATION_MINSIZEREL "${BADNESS_RELEASE_LIBRARY}"
    )
endif()

add_executable(badness_example main.cpp)
add_dependencies(badness_example badness_rust_ffi)
target_compile_features(badness_example PRIVATE cxx_std_20)
target_include_directories(badness_example PRIVATE "${BADNESS_ROOT}/include")
target_link_libraries(badness_example PRIVATE badness_ffi)

if(BUILD_TESTING)
    add_executable(badness_test test.cpp)
    add_dependencies(badness_test badness_rust_ffi)
    target_compile_features(badness_test PRIVATE cxx_std_20)
    target_include_directories(badness_test PRIVATE "${BADNESS_ROOT}/include")
    target_link_libraries(badness_test PRIVATE badness_ffi)
    add_test(NAME badness_test COMMAND badness_test)
endif()

# Embed the linked Rust library's config-specific directory as a development rpath.
# Packaging should install the library beside its consumer instead.
if(APPLE OR UNIX)
    set_target_properties(badness_example PROPERTIES
        BUILD_RPATH "$<TARGET_FILE_DIR:badness_ffi>"
    )
    if(BUILD_TESTING)
        set_target_properties(badness_test PROPERTIES
            BUILD_RPATH "$<TARGET_FILE_DIR:badness_ffi>"
        )
    endif()
endif()
