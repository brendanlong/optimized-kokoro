# Builds C espeak-ng 1.52.0's library code statically into the oracle driver (ttp) and the timing
# driver (bench). Test tooling only; nothing here ships.
VERSION = 1.52.0
SRC = espeak-ng-$(VERSION)
S = $(SRC)/src
LIB = common mnemonics error ieee80 compiledata compiledict dictionary encoding intonation langopts numbers phoneme phonemelist readclause setlengths soundicon spect ssml synthdata synthesize tr_languages translate translateword voices wavegen speech espeak_api
UCD = case categories ctype proplist scripts tostring
OBJS = $(addprefix obj/,$(addsuffix .o,$(LIB))) $(addprefix obj/ucd_,$(addsuffix .o,$(UCD)))
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

$(filter-out obj/ucd_%,$(OBJS)): obj/%.o: $(S)/libespeak-ng/%.c
	@mkdir -p obj
	$(CC) $(CF) -c $< -o $@

$(filter obj/ucd_%,$(OBJS)): obj/ucd_%.o: $(S)/ucd-tools/src/%.c
	@mkdir -p obj
	$(CC) $(CF) -c $< -o $@

ttp: ttp.c $(OBJS)
	$(CC) $(CF) ttp.c $(OBJS) -lm -o $@

bench: bench.c $(OBJS)
	$(CC) $(CF) bench.c $(OBJS) -lm -o $@

# Regenerates crates/espeak-ng/src/ucd_tables.rs from ucd-tools.
ucddump: ucddump.c $(OBJS)
	$(CC) $(CF) ucddump.c $(OBJS) -lm -o $@

clean:
	rm -rf obj ttp bench ucddump

.PHONY: all clean
