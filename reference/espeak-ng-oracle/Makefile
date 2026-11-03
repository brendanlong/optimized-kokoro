# Builds C espeak-ng 1.52.0's library code statically into the oracle driver (ttp) and the timing
# driver (bench). Test tooling only; nothing here ships.
VERSION = 1.52.0
SRC = espeak-ng-$(VERSION)
S = $(SRC)/src
LIB = common mnemonics error ieee80 compiledata compiledict dictionary encoding intonation langopts numbers phoneme phonemelist readclause setlengths soundicon spect ssml synthdata synthesize tr_languages translate translateword voices wavegen speech espeak_api
UCD = case categories ctype proplist scripts tostring
OBJ ?= obj
OBJS = $(addprefix $(OBJ)/,$(addsuffix .o,$(LIB))) $(addprefix $(OBJ)/ucd_,$(addsuffix .o,$(UCD)))
CFLAGS ?= -O3 -g
# include/compat maps iswalpha() etc. to ucd-tools, as espeak-ng's own build does.
CF = $(CFLAGS) -fwrapv -Iinclude -I$(S)/include -I$(S)/include/compat -I$(S)/ucd-tools/src/include -I$(S)/libespeak-ng -DLIBESPEAK_NG_EXPORT=1 -DPATH_ESPEAK_DATA=\"/nonexistent\"

all: ttp bench

$(SRC).tar.gz:
	curl -fL -o $@ https://github.com/espeak-ng/espeak-ng/archive/refs/tags/$(VERSION).tar.gz

$(SRC)/.extracted: $(SRC).tar.gz
	tar xzf $<
	touch $@

$(S)/%.c: $(SRC)/.extracted ;
$(S)/libespeak-ng/translate.c: $(SRC)/.extracted ;

$(filter-out $(OBJ)/ucd_%,$(OBJS)): $(OBJ)/%.o: $(S)/libespeak-ng/%.c
	@mkdir -p $(OBJ)
	$(CC) $(CF) -c $< -o $@

$(filter $(OBJ)/ucd_%,$(OBJS)): $(OBJ)/ucd_%.o: $(S)/ucd-tools/src/%.c
	@mkdir -p $(OBJ)
	$(CC) $(CF) -c $< -o $@

ttp: ttp.c $(OBJS)
	$(CC) $(CF) ttp.c $(OBJS) -lm -o $@

# Builds for finding calls where C reads out of bounds (ASan, continuing after each report) or
# reads uninitialised memory (run under valgrind's memcheck). `oracle.py pairs --ub` uses both.
# VALGRIND_CFLAGS=-I.../include if valgrind's headers aren't installed system-wide.
sanitized: ttp-asan ttp-memcheck

ttp-asan: ttp.c $(S)/libespeak-ng/translate.c
	$(MAKE) OBJ=obj-asan CFLAGS="-O1 -g -fno-omit-frame-pointer -fsanitize=address -fsanitize-recover=address" obj-asan/ttp
	cp obj-asan/ttp $@

ttp-memcheck: ttp.c $(S)/libespeak-ng/translate.c
	$(MAKE) OBJ=obj-memcheck CFLAGS="-O1 -g $(VALGRIND_CFLAGS)" obj-memcheck/ttp
	cp obj-memcheck/ttp $@

$(OBJ)/ttp: ttp.c $(OBJS)
	$(CC) $(CF) ttp.c $(OBJS) -lm -o $@

bench: bench.c $(OBJS)
	$(CC) $(CF) bench.c $(OBJS) -lm -o $@

# Regenerates crates/espeak-ng/src/ucd_tables.rs from ucd-tools.
ucddump: ucddump.c $(OBJS)
	$(CC) $(CF) ucddump.c $(OBJS) -lm -o $@

clean:
	rm -rf obj obj-asan obj-memcheck ttp ttp-asan ttp-memcheck bench ucddump

.PHONY: all clean sanitized
