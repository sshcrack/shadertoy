out vec2 vUv;

void main() {
    vUv = a_grid;
    vec2 position = a_grid * 2.0 - 1.0;
    gl_Position = vec4(position, 0.0, 1.0);
}
