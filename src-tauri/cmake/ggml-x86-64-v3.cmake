# Jeu d'instructions de ggml (whisper.cpp) pour les binaires publiés en x86-64.
#
# Inclus avant chaque project() via CMAKE_PROJECT_INCLUDE_BEFORE (voir .cargo/config.toml) :
# whisper-rs-sys 0.14 ne transmet à CMake que les variables WHISPER_* et CMAKE_*, pas GGML_*.
#
# Pourquoi : la CI définit SOURCE_DATE_EPOCH (build reproductible). ggml en déduit
# GGML_NATIVE_DEFAULT=OFF, ce qui coupe aussi SSE4.2, AVX, AVX2, BMI2, FMA et F16C
# (`if (GGML_NATIVE OR NOT GGML_NATIVE_DEFAULT) set(INS_ENB OFF)`) : moteur en SSE2 scalaire,
# 10 à 30 fois plus lent sur processeur. On rétablit ici le niveau x86-64-v3, celui de la
# variante « haswell » de ggml : Intel depuis Haswell (2013), AMD depuis Excavator (2015)/Zen.
# Le moteur local vérifie au chargement que le processeur dispose bien de ces instructions
# (`engine::local::cpu_check`) et refuse proprement de démarrer sinon (pas d'« illegal instruction »).
#
# Builds de développement (sans SOURCE_DATE_EPOCH) : rien ne change, ggml reste en -march=native.
if(DEFINED ENV{SOURCE_DATE_EPOCH} AND CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|AMD64|amd64|x64)$")
    set(GGML_NATIVE OFF CACHE BOOL "ggml: optimize the build for the current system" FORCE)
    foreach(feature IN ITEMS GGML_SSE42 GGML_AVX GGML_AVX2 GGML_BMI2 GGML_FMA GGML_F16C)
        set(${feature} ON CACHE BOOL "ggml: x86-64-v3 (binaire portable)" FORCE)
    endforeach()
endif()
