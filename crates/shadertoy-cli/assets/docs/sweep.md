Parameter sweeps render deterministic visual variants without temporary project
copies or shell loops.

Scalar/int/bool values use comma-separated alternatives:

  shadertoy sweep --frame 120 --set u_foam_gain=0.8,1.0,1.2

Repeat --set to build a Cartesian product:

  shadertoy sweep --frame 120 \
    --set u_foam_gain=0.8,1.0,1.2 \
    --set u_spray_steps=6,8,10

Vector components already use commas, so separate vector alternatives with
semicolons (quote the shell argument):

  shadertoy sweep --set 'wind=1,0;0.7,0.7;0,1'

Each variant starts from fresh deterministic state. Variant PNGs are written to
PROJECT/target/sweep by default and a contact-sheet.png is generated there.
Use --output-dir, --contact-sheet, --columns, or --no-contact-sheet to adjust
the outputs. --pass can compare a named 2D buffer/compute/mesh pass instead of Image.

The command caps Cartesian expansion at 256 variants to avoid accidental
explosive renders. All sweep values are parsed and range-checked using the
project's declared [[uniform]] definitions.

For bias-resistant visual selection, add --blind:

  shadertoy sweep --blind --frame 120 --set u_foam_gain=0.8,1.0,1.2

This randomizes/anonymizes variants as A/B/C, writes a blind contact sheet, and
seals the A/B/C-to-parameter mapping until a judgment is recorded. Continue with
shadertoy blind judge, then shadertoy blind reveal. See shadertoy docs blind.
