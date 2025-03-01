###############################################################################
# Configuration
###############################################################################

# Platform detection.
ifeq ($(OS),Windows_NT)
	PLATFORM = Windows
else
	UNAME_S := $(shell uname -s)
	ifeq ($(UNAME_S),Darwin)
		PLATFORM = Darwin
	else
		PLATFORM = Linux
	endif
endif

# Some commands need xcode.
ifeq ($(PLATFORM),Darwin)
	XCRUN = xcrun
endif

# The base compiler flags. More can be added on command line.
CFLAGS += -I../inc -O2 -Wall -Wextra

# Cross-platform compilation settings.
ifeq ($(PLATFORM),Windows)
	CC = gcc
	CFLAGS += -D_CRT_SECURE_NO_WARNINGS
else
	CC = clang
	CFLAGS += -fPIC -Werror
endif

# Settings for blst.
BLST_LIBRARY = ../lib/libblst.a
BLST_BUILDSCRIPT = ../blst/build.sh
BLST_BUILDSCRIPT_FLAGS = -D__BLST_PORTABLE__

# Libraries to build with.
LIBS = $(BLST_LIBRARY)

###############################################################################
# Core
###############################################################################

all: c_kzg_4844.o test

$(BLST_BUILDSCRIPT):
	@git submodule update --init

$(BLST_LIBRARY): $(BLST_BUILDSCRIPT)
	@cd $(dir $(BLST_BUILDSCRIPT)) && \
	./$(notdir $(BLST_BUILDSCRIPT)) $(BLST_BUILDSCRIPT_FLAGS) && \
	cp $(notdir $(BLST_LIBRARY)) ../lib && \
	cp bindings/*.h ../inc

.PHONY: blst
blst: $(BLST_LIBRARY)

c_kzg_4844.o: c_kzg_4844.c $(BLST_LIBRARY)
	@$(CC) $(CFLAGS) -c $<

test_c_kzg_4844: CFLAGS += -O0
test_c_kzg_4844: test_c_kzg_4844.c c_kzg_4844.c $(BLST_LIBRARY)
	@$(CC) $(CFLAGS) -o $@ $< $(LIBS)

.PHONY: test
test: test_c_kzg_4844
	@./test_c_kzg_4844

###############################################################################
# Coverage
###############################################################################

test_c_kzg_4844_cov: CFLAGS += -O0 -fprofile-instr-generate -fcoverage-mapping
test_c_kzg_4844_cov: test_c_kzg_4844.c c_kzg_4844.c
	@$(CC) $(CFLAGS) -o $@ $< $(LIBS)

.PHONY: coverage
coverage: test_c_kzg_4844_cov
	@LLVM_PROFILE_FILE="ckzg.profraw" ./$<
	@$(XCRUN) llvm-profdata merge --sparse ckzg.profraw -o ckzg.profdata
	@$(XCRUN) llvm-cov show --instr-profile=ckzg.profdata --format=html \
	    $< c_kzg_4844.c > coverage.html
	@$(XCRUN) llvm-cov report --instr-profile=ckzg.profdata \
	    --show-functions $< c_kzg_4844.c

###############################################################################
# Profile
###############################################################################

test_c_kzg_4844_prof: LIBS += -lprofiler
test_c_kzg_4844_prof: CFLAGS += -O0 -DPROFILE
ifeq ($(PLATFORM),Darwin)
test_c_kzg_4844_prof: CFLAGS += -L$(shell brew --prefix gperftools)/lib
test_c_kzg_4844_prof: CFLAGS += -I$(shell brew --prefix gperftools)/include
endif
test_c_kzg_4844_prof: test_c_kzg_4844.c c_kzg_4844.c
	@$(CC) $(CFLAGS) -o $@ $< $(LIBS)

.PHONY: run_profiler
run_profiler: test_c_kzg_4844_prof
	@CPUPROFILE_FREQUENCY=1000000000 ./$<

.PHONY: profile_%
profile_%: run_profiler
	@echo Profiling $*...
	@pprof --pdf --nodefraction=0.00001 --edgefraction=0.00001 \
	    ./test_c_kzg_4844_prof $*.prof > $*.pdf

.PHONY: profile
profile: \
	profile_blob_to_kzg_commitment \
	profile_compute_kzg_proof \
	profile_compute_blob_kzg_proof \
	profile_verify_kzg_proof \
	profile_verify_blob_kzg_proof \
	profile_verify_blob_kzg_proof_batch

###############################################################################
# Sanitize
###############################################################################

.PHONY: sanitize_%
sanitize_%: CFLAGS += -O0 -fsanitize=$*
sanitize_%: test_c_kzg_4844.c c_kzg_4844.c
	@echo Running sanitize=$*...
	@$(CC) $(CFLAGS) -o $@ $< $(LIBS)
	@ASAN_OPTIONS=allocator_may_return_null=1 \
	    LSAN_OPTIONS=allocator_may_return_null=1 \
	    ./$@; rm $@

.PHONY: sanitize
ifeq ($(PLATFORM),Darwin)
sanitize: \
	sanitize_address \
	sanitize_undefined
else ifeq ($(PLATFORM),Linux)
sanitize: \
	sanitize_address \
	sanitize_leak \
	sanitize_safe-stack \
	sanitize_undefined
endif

###############################################################################
# Analyze
###############################################################################

.PHONY: analyze
analyze: c_kzg_4844.c
	@$(CC) --analyze -Xanalyzer -analyzer-output=html \
	    -o analysis-report $(CFLAGS) -c $<
	@[ -d analysis-report ] && exit 1 || exit 0

###############################################################################
# Cleanup
###############################################################################

.PHONY: format
format:
	@clang-format -i --sort-includes c_kzg_4844.* test_c_kzg_4844.c

.PHONY: clean
clean:
	@rm -f *.o *.profraw *.profdata *.html xray-log.* *.prof *.pdf \
	    test_c_kzg_4844 test_c_kzg_4844_cov test_c_kzg_4844_prof
	@rm -rf analysis-report
