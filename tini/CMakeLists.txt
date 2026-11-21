cmake_minimum_required(VERSION 3.16)
project(cdelivery_tini C)

file(STRINGS "${CMAKE_CURRENT_SOURCE_DIR}/VERSION" TINI_RELEASE LIMIT_COUNT 1)
configure_file(src/tiniConfig.h.in tiniConfig.h @ONLY)
configure_file(vendor/tini/src/tini.c tini.c COPYONLY)
execute_process(
    COMMAND patch --batch -p1 -i "${CMAKE_CURRENT_SOURCE_DIR}/patches/secret-env.patch"
    WORKING_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}"
    RESULT_VARIABLE PATCH_RESULT
)
if(NOT PATCH_RESULT EQUAL 0)
    message(FATAL_ERROR "Unable to apply Tini extension")
endif()

add_executable(tini "${CMAKE_CURRENT_BINARY_DIR}/tini.c" src/secret-env.c)
target_include_directories(tini PRIVATE "${CMAKE_CURRENT_BINARY_DIR}" src vendor/tini/src)
target_compile_features(tini PRIVATE c_std_99)
target_compile_definitions(tini PRIVATE _FORTIFY_SOURCE=2)
target_compile_options(tini PRIVATE -Os -Wall -Wextra -Werror -Wformat -fstack-protector-strong)
target_link_options(tini PRIVATE -static -Wl,-z,relro,-z,now -s)
