# SPDX-License-Identifier: MIT

cmake_minimum_required(VERSION 3.24)

project(claude-crab VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

include(GNUInstallDirs)
include(CTest)

find_package(Qt6 6.6 REQUIRED COMPONENTS Core Gui Qml Quick)
find_package(LayerShellQt REQUIRED)

qt_standard_project_setup(REQUIRES 6.6)

# --- generated sprite sheet -------------------------------------------------
# gen_sprites.py is the source of truth; the PNG is a build artefact and is not
# checked in, so art and manifest can never drift apart.

find_package(Python3 REQUIRED COMPONENTS Interpreter)

set(ASSET_DIR ${CMAKE_CURRENT_BINARY_DIR}/assets)
set(SPRITE_SHEET ${ASSET_DIR}/spritesheet.png)
set(SPRITE_SHEET_FANCY ${ASSET_DIR}/spritesheet-fancy.png)
set(SPRITE_MANIFEST ${ASSET_DIR}/manifest.json)

add_custom_command(
    OUTPUT ${SPRITE_SHEET} ${SPRITE_SHEET_FANCY} ${SPRITE_MANIFEST}
    COMMAND ${Python3_EXECUTABLE} ${CMAKE_CURRENT_SOURCE_DIR}/tools/gen_sprites.py
            --out ${ASSET_DIR}
    DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/tools/gen_sprites.py
    COMMENT "Generating crab sprite sheet"
    VERBATIM
)
add_custom_target(sprites DEPENDS ${SPRITE_SHEET} ${SPRITE_SHEET_FANCY} ${SPRITE_MANIFEST})

# --- library ----------------------------------------------------------------
# The state machine lives in its own target so the tests can link it without
# dragging in QtQuick or layer-shell.

add_library(crabcore STATIC
    src/SessionTracker.cpp
    src/SessionTracker.h
    src/CrabConfig.cpp
    src/CrabConfig.h
    src/CrabController.cpp
    src/CrabController.h
)
target_include_directories(crabcore PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/src)
# Quick is here only for QQuickWindow::setMask in CrabController; everything
# else in this library is Core-only and headless-testable.
target_link_libraries(crabcore PUBLIC Qt6::Core Qt6::Quick)

# --- application ------------------------------------------------------------

qt_add_executable(claude-crab src/main.cpp)
add_dependencies(claude-crab sprites)

# Set before qt_add_qml_module: it resolves aliases at declaration time. Keeps
# the resource paths at qrc:/qt/qml/ClaudeCrab/assets/... regardless of where
# the build tree puts the generated files.
set_source_files_properties(${SPRITE_SHEET} PROPERTIES QT_RESOURCE_ALIAS assets/spritesheet.png)
set_source_files_properties(${SPRITE_SHEET_FANCY} PROPERTIES QT_RESOURCE_ALIAS assets/spritesheet-fancy.png)
set_source_files_properties(${SPRITE_MANIFEST} PROPERTIES QT_RESOURCE_ALIAS assets/manifest.json)

qt_add_qml_module(claude-crab
    URI ClaudeCrab
    VERSION 1.0
    QML_FILES
        qml/Main.qml
        qml/Crab.qml
        qml/CrabBrain.qml
        qml/CrabMenu.qml
    RESOURCES
        ${SPRITE_SHEET}
        ${SPRITE_SHEET_FANCY}
        ${SPRITE_MANIFEST}
    RESOURCE_PREFIX /qt/qml
)

target_link_libraries(claude-crab PRIVATE
    crabcore
    Qt6::Gui
    Qt6::Qml
    Qt6::Quick
    LayerShellQt::Interface
)

# --- tests ------------------------------------------------------------------

if(BUILD_TESTING)
    find_package(Qt6 REQUIRED COMPONENTS Test)

    qt_add_executable(tst_sessiontracker tests/tst_sessiontracker.cpp)
    target_link_libraries(tst_sessiontracker PRIVATE crabcore Qt6::Test)
    add_test(NAME sessiontracker COMMAND tst_sessiontracker)

    qt_add_executable(tst_crabconfig tests/tst_crabconfig.cpp)
    target_link_libraries(tst_crabconfig PRIVATE crabcore Qt6::Test)
    add_test(NAME crabconfig COMMAND tst_crabconfig)

    qt_add_executable(tst_crabcontroller tests/tst_crabcontroller.cpp)
    target_link_libraries(tst_crabcontroller PRIVATE crabcore Qt6::Test)
    add_test(NAME crabcontroller COMMAND tst_crabcontroller)

    add_test(
        NAME crab_hooks
        COMMAND ${Python3_EXECUTABLE} -m pytest -q
                ${CMAKE_CURRENT_SOURCE_DIR}/tests/test_crab_hooks.py
    )
    add_test(
        NAME gen_sprites
        COMMAND ${Python3_EXECUTABLE} -m pytest -q
                ${CMAKE_CURRENT_SOURCE_DIR}/tests/test_gen_sprites.py
    )
endif()

# --- install ----------------------------------------------------------------

set(CLAUDE_CRAB_BINARY ${CMAKE_INSTALL_FULL_BINDIR}/claude-crab)
configure_file(packaging/claude-crab.service.in
               ${CMAKE_CURRENT_BINARY_DIR}/claude-crab.service @ONLY)

# systemd only searches a fixed set of directories for user units, and
# <prefix>/lib/systemd/user is not among them for a prefix inside $HOME. Pick
# the right one rather than installing somewhere systemd will never look.
if(NOT DEFINED SYSTEMD_USER_UNIT_DIR)
    if(CMAKE_INSTALL_PREFIX MATCHES "^$ENV{HOME}")
        set(SYSTEMD_USER_UNIT_DIR ${CMAKE_INSTALL_DATADIR}/systemd/user)
    else()
        set(SYSTEMD_USER_UNIT_DIR ${CMAKE_INSTALL_LIBDIR}/systemd/user)
    endif()
endif()

install(TARGETS claude-crab RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(PROGRAMS tools/crab_hooks.py
        DESTINATION ${CMAKE_INSTALL_BINDIR}
        RENAME claude-crab-hooks)
install(FILES ${CMAKE_CURRENT_BINARY_DIR}/claude-crab.service
        DESTINATION ${SYSTEMD_USER_UNIT_DIR})
install(FILES packaging/dev.quinnjr.claude-crab.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
