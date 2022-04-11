# Codename DW-666

An embedded MIDI routing app to interface an [Arturia Beatstep](https://www.arturia.com/products/hybrid-synths/beatstep)
with a [Korg DW-6000](https://www.vintagesynth.com/korg/dw6000.php).

Provides immediate control and modern tactile UI over the sound parameters of a vintage-but-knobless synthesizer.

Targets STM32F4 "blackpill" board. Uses RTIC as a real-time software platform.

Still under development, probably forever. It all worked for some time, before I started to try and make it better [le sigh]

## Interface

Each Beatstep knobs controls the value of a parameter of the DW6000. 

The big top left knob controls the filter cutoff to sweeeep those sweeeet NJM2069s **anytime**. 

The small top right knob _always_ controls the filter becauPEWPEWPEW.

The top right 4 pads control some on/off parameters like Chorus and ??? (see code)

### Parameter pages

There are 15 knobs left but more than 50 parameters! Parameters are thus grouped in pages. 

The top left 4 pads on the Beatstep which the parameter page is active. 

**Quick tap** on a pad to switch to the associated page. 

**Hold down** a pad to quick-edit that page's parameters, then **release** to go back to the previous page

[[INSERT HERE: A nice markdown table showing map of pages, knobs and parameters.]] 

### Quick patch change

Hold down one of the 8 lower pad and then tap on a upper pad.

8 pads (low) x 8 pads (high) = **64 combinations**

Number of patches on the DW-6000? **64**

Coincidence? _I think not._

## TODO

- Make it run again, dog magnit!

- Still requires an external computer to route USB MIDI bewteen Beatstep and board. USB MIDI host co-board (using Atmel
  SAM D21) undergoing development in a separate project. Meanwhile, ALSA's `aconnect` is a friend.

- An LCD screen to display current patch values would be nice. Attempts to use ILI9341 have failed up to here, halp.

- Using async Rust would make some code much cleaner (callbacks, uh). USB Host project (see above) might provide answers.

- Use native Beatstep sequences to drive an arpeggiator

- Make that external LFO2 thingie better harder stronger faster and document it too

- Record a small video of the whole thing in action

- Make music, not softwar!
