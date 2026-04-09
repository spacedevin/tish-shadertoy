// NOTE: You have to wait for about 5 second for the music to start!
// This file is released under CC0 1.0 Universal (Public Domain Dedication).
// To the extent possible under law, mrange has waived all copyright
// and related or neighboring rights to this work.
// See <https://creativecommons.org/publicdomain/zero/1.0/> for details.

// If you suffer from epilepsy, this shader might not be the best choice for you. I’d hate for it to cause any issues!

// NOTE: You have to wait for about 5 second for the music to start!

// This is a multipass shader
//  Buffer A - Preprocessing of FFT and beat detection. Does this help perf? Not sure, fun to experiment with
//  Image    - Main effect

// Hopefully my preprocessor buffer experiment don't mess up the effect for you. I guess it will show in the comments.

const float
  PI            =3.141592654
, TAU           =2.*PI
, PI_2          =.5*PI
, base_color        =6.
, rot_speed         =.1
, spectrum_boost    =1.
, speed             =-3.
, sway              =.707
, sway_speed        =.125
, sway_dist         =2.
;

#define ROT(a)      mat2(cos(a), sin(a), -sin(a), cos(a))

float spectrum(vec2 r, float x) {
  return texelFetch(iChannel0, ivec2(x*r.x,0.), 0).x;
}

float bass_thump(vec2 r) {
  return texelFetch(iChannel0, ivec2(0.,2.), 0).x;
}

// License: Unknown, author: Claude Brezinski, found: https://mathr.co.uk/blog/2017-09-06_approximating_hyperbolic_tangent.html
vec3 tanh_approx(vec3 x) {
  // Attempt at cheap tanh. Keeps values in [-1, 1] range without the cost of
  // the real tanh. Used here for soft tone mapping in post-processing.

  // Main motivation is that tanh on Macbook M's don't like large input values

  vec3 x2 = x*x;
  return clamp(x*(27.0 + x2)/(27.0+9.0*x2), -1.0, 1.0);
}

// License: MIT, author: Inigo Quilez, found: https://iquilezles.org/articles/intersectors/
float ray_cylinder(vec3 ro, vec3 rd, float r) {
  // Intersects a ray with an infinite cylinder of radius r centered on the z-axis.
  // Returns the ray parameter t at the hit point, or -1 if missed.

  float
      a=dot(rd.xy, rd.xy)
    , b=dot(ro.xy, rd.xy)
    , c=dot(ro.xy, ro.xy) - r*r
    , h=b*b - a*c
    , t
    ;

  if(h<.0) return -1.;

  h = sqrt(h);
  t = (-b+h)/a;

  return t;
}

// License: MIT, author: Pascal Gilcher, found: https://www.shadertoy.com/view/flSXRV
float atan_approx(float y, float x) {
  // Fast atan2 approximation. Cheaper than atan() on some GPUs.

  float
    cosatan2 = x/(abs(x)+abs(y))
  , t = PI_2-cosatan2*PI_2
  ;
  return y<0.?-t:t;
}

// License: MIT OR CC-BY-NC-4.0, author: mercury, found: https://mercury.sexy/hg_sdf/
vec2 mod_polar(inout vec2 p, float repetitions) {
  // Divides the xy-plane into angular slices around the origin.
  // Folds p into a single slice and returns (slice index, radius).
  float
    angle = TAU/repetitions
  , a = atan_approx(p.y, p.x) + angle/2.
  , r = length(p)
  , c = floor(a/angle)
  ;
  a = mod(a,angle) - angle/2.;
  p = vec2(cos(a), sin(a))*r;
  if (abs(c) >= (repetitions/2.)) c = abs(c);
  return vec2(c,r);
}

// License: Unknown, author: Unknown, found: don't remember
float hash(vec2 co) {
  // Produces a pseudo-random number based on co

  co+=.1234;
  return fract(sin(dot(co.xy ,vec2(12.9898,58.233)))*13758.5453);
}

// Distance to a line segment along z-axis with half-length hl.
float segmentz(vec3 p, float hl) {
  // Returns distance to the segment surface (treating it as a capsule-like shape).

  p.z=abs(p.z)-hl;
  float
    d0=length(p)       // distance to segment endpoint (sphere cap)
  , d1=length(p.xy)    // distance to segment body (cylinder)
  ;

  // If past the endpoint, use sphere distance; otherwise cylinder distance
  return p.z>0.?d0:d1;
}

vec3 hyperspace(vec2 r, vec3 RO, vec3 RD, float FO) {
  // The main effect: concentric cylinders of glowing line segments
  // arranged in a tunnel-like hyperspace.

  // RO: ray origin, RD: ray direction, FO: fog/glow falloff factor
  float
    H0
  , H1
  , REP
  , ci
  , d
  , n1
  , fo
  ;

  vec2
    N
  ;

  vec3
    // Base glow that pulses with bass. The division by (1+1e-3-RD.z+RD.x*RD.x)
    // makes rays looking straight down the tunnel brighter.
    o=mix(0e-4,3e-4, bass_thump(r))*(vec3(1,4,16))/(1.+1e-3-(RD.z)+RD.x*RD.x)
  , p
  , c
  ;
  vec4
    O
  ;

  // Rotation matrix for the cylinder
  mat2
    R=ROT(rot_speed*iTime)
  ;

  // Loop over concentric cylinders at increasing radii
  for (float j=2.;j<9.;++j) {
    REP=j*j+3.;  // more segments on larger cylinders
    ci=ray_cylinder(RO,RD,j);  // find where the ray hits this cylinder
    p=ci*RD+RO;  // hit position on cylinder surface
    p.xy*=R;     // rotate the hit point
    N=mod_polar(p.xy,REP);  // fold into one angular slice; N.x=slice, N.y=radius

    // Per-slice random value, seeded by cylinder and slice index
    H0=hash(.123*vec2(j,N.x));
    // Scroll along z at a speed that varies per slice
    p.z-=speed*iTime*(1.+H0*H0*H0);
    // Distance-based fog: far-away cylinders contribute less light
    fo=exp(-2e-3*ci*ci);
    // Offset from the cell center on the cylinder surface
    c=vec3(N.y,0,p.z);
    // Split into cells along z-axis
    c-=vec3(p.xy,floor(p.z+.5));

    // Check the 3 nearest z-cells: previous, current, next
    //  This to reduce visual artifacts when colors from one cell leaks into
    //  the other
    for(float i=-1.;i<=1.;++i) {
      n1=floor(i+p.z+.5);
      // Per-segment random value
      H1=hash(vec2(H0,n1));
      // Color derived from hash, shifted by base_color uniform
      O=1.+sin(PI*H1+vec4(0,1,8,4)-base_color);
      // Distance to the line segment in this cell
      d=segmentz(c-vec3(0,0,i),.35*H1*H1+.1);
      // Audio reactivity: spectrum lookup maps each segment to a frequency band
      H1=spectrum_boost*spectrum(r, mix(.975,.025,H1));
      // Accumulate glow: inverse-distance falloff, modulated by
      // audio, fog, and per-segment color
      o+=
          5e-3
        * H1
        / max(d,4e-4*ci+.02*FO)
        * fo
        * (O.w+.1)
        * O.xyz;
    }
  }

  return o;
}

void mainImage(out vec4 O, vec2 C) {
//#define SHOW_BUFFER_A 
#ifdef SHOW_BUFFER_A
  O=texelFetch(iChannel1, ivec2(C), 0);

#else
  vec2
    r=iResolution.xy
  , q=C/r
  , p=(2.*q-1.)*r/r.y  // map to aspect-corrected coordinates centered at origin
  , t2=sway_speed*iTime*vec2(sqrt(2.),1.)
  ;

  vec3
    RO=vec3(sway*sin(t2),-sway_dist)
  , LA=vec3(0)
  , Z =normalize(LA-RO)
  , X =normalize(cross(Z,vec3(.2*cos(t2)+vec2(0,1.),0)))
  , Y =cross(X,Z)
  , RD=normalize(2.*Z+p.y*Y-p.x*X)
  , o =vec3(0)
  ;

  o=hyperspace(r, RO,RD,0.);

  // Post-processing chain
  o-=3e-2*vec3(3,2,1)*length(p+.25);     // subtle warm-toned gradient
  o=max(o,0.);
  o=tanh_approx(1.125*o);                // soft tone mapping (keeps highlights from clipping)
  o=sqrt(o)-.05;                          // gamma correction with slight black lift

  O=vec4(o,1);
#endif
}
