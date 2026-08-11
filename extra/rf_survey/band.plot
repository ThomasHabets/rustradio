#!/usr/bin/env gnuplot
#
# Example:
#
# ```
# uhd_rf_survey --start-frequency 5.15G --stop-frequency 5.895G --output power-5GHz.txt --sample-rate 8M --summarize
# [… press Ctrl-C after a while …]
# gnuplot -e 'survey_data="power-2.4GHz.txt"' band.plot
# ```
#

set terminal png truecolor rounded size 1920,720 enhanced
if (!exists("survey_data")) survey_data = "band.txt"
if (!exists("db_output")) db_output = "wifi-db.png"
if (!exists("power_output")) power_output = "wifi-power.png"
set autoscale xfix
set xtics 0.1
set mxtics 10
set grid xtics
set grid ytics
set format x "%.2fGHz"
set key outside bottom center horizontal

# 2.4 GHz
set object  1 rectangle from first 2.401, graph 0 to 2.423,graph 1 fs solid fc rgb "#ffd0d0" back
set object  6 rectangle from first 2.426, graph 0 to 2.448,graph 1 fs solid fc rgb "#d0ffd0" back
set object 11 rectangle from first 2.451, graph 0 to 2.473,graph 1 fs solid fc rgb "#d0d0ff" back
set label  1 center at first 2.412, graph 0.95 "Channel 1"  font ",14"
set label  6 center at first 2.436, graph 0.95 "Channel 6"  font ",14"
set label 11 center at first 2.461, graph 0.95 "Channel 11" font ",14"

# 5 GHz Wi-Fi, 20 MHz channels

set object  32 rectangle from first 5.150, graph 0 to 5.170, graph 1 fs solid fc rgb "#d0d0ff" back
set label   32 center at first 5.160, graph 0.95 "32" font ",14"

set object  36 rectangle from first 5.170, graph 0 to 5.190, graph 1 fs solid fc rgb "#ffd0d0" back
set label   36 center at first 5.180, graph 0.95 "36" font ",14"

set object  40 rectangle from first 5.190, graph 0 to 5.210, graph 1 fs solid fc rgb "#d0ffd0" back
set label   40 center at first 5.200, graph 0.95 "40" font ",14"

set object  44 rectangle from first 5.210, graph 0 to 5.230, graph 1 fs solid fc rgb "#d0d0ff" back
set label   44 center at first 5.220, graph 0.95 "44" font ",14"

set object  48 rectangle from first 5.230, graph 0 to 5.250, graph 1 fs solid fc rgb "#ffd0d0" back
set label   48 center at first 5.240, graph 0.95 "48" font ",14"

set object  52 rectangle from first 5.250, graph 0 to 5.270, graph 1 fs solid fc rgb "#d0ffd0" back
set label   52 center at first 5.260, graph 0.95 "52" font ",14"

set object  56 rectangle from first 5.270, graph 0 to 5.290, graph 1 fs solid fc rgb "#d0d0ff" back
set label   56 center at first 5.280, graph 0.95 "56" font ",14"

set object  60 rectangle from first 5.290, graph 0 to 5.310, graph 1 fs solid fc rgb "#ffd0d0" back
set label   60 center at first 5.300, graph 0.95 "60" font ",14"

set object  64 rectangle from first 5.310, graph 0 to 5.330, graph 1 fs solid fc rgb "#d0ffd0" back
set label   64 center at first 5.320, graph 0.95 "64" font ",14"

set object 100 rectangle from first 5.490, graph 0 to 5.510, graph 1 fs solid fc rgb "#d0d0ff" back
set label  100 center at first 5.500, graph 0.95 "100" font ",14"

set object 104 rectangle from first 5.510, graph 0 to 5.530, graph 1 fs solid fc rgb "#ffd0d0" back
set label  104 center at first 5.520, graph 0.95 "104" font ",14"

set object 108 rectangle from first 5.530, graph 0 to 5.550, graph 1 fs solid fc rgb "#d0ffd0" back
set label  108 center at first 5.540, graph 0.95 "108" font ",14"

set object 112 rectangle from first 5.550, graph 0 to 5.570, graph 1 fs solid fc rgb "#d0d0ff" back
set label  112 center at first 5.560, graph 0.95 "112" font ",14"

set object 116 rectangle from first 5.570, graph 0 to 5.590, graph 1 fs solid fc rgb "#ffd0d0" back
set label  116 center at first 5.580, graph 0.95 "116" font ",14"

set object 120 rectangle from first 5.590, graph 0 to 5.610, graph 1 fs solid fc rgb "#d0ffd0" back
set label  120 center at first 5.600, graph 0.95 "120" font ",14"

set object 124 rectangle from first 5.610, graph 0 to 5.630, graph 1 fs solid fc rgb "#d0d0ff" back
set label  124 center at first 5.620, graph 0.95 "124" font ",14"

set object 128 rectangle from first 5.630, graph 0 to 5.650, graph 1 fs solid fc rgb "#ffd0d0" back
set label  128 center at first 5.640, graph 0.95 "128" font ",14"

set object 132 rectangle from first 5.650, graph 0 to 5.670, graph 1 fs solid fc rgb "#d0ffd0" back
set label  132 center at first 5.660, graph 0.95 "132" font ",14"

set object 136 rectangle from first 5.670, graph 0 to 5.690, graph 1 fs solid fc rgb "#d0d0ff" back
set label  136 center at first 5.680, graph 0.95 "136" font ",14"

set object 140 rectangle from first 5.690, graph 0 to 5.710, graph 1 fs solid fc rgb "#ffd0d0" back
set label  140 center at first 5.700, graph 0.95 "140" font ",14"

set object 144 rectangle from first 5.710, graph 0 to 5.730, graph 1 fs solid fc rgb "#d0ffd0" back
set label  144 center at first 5.720, graph 0.95 "144" font ",14"

set object 149 rectangle from first 5.735, graph 0 to 5.755, graph 1 fs solid fc rgb "#d0d0ff" back
set label  149 center at first 5.745, graph 0.95 "149" font ",14"

set object 153 rectangle from first 5.755, graph 0 to 5.775, graph 1 fs solid fc rgb "#ffd0d0" back
set label  153 center at first 5.765, graph 0.95 "153" font ",14"

set object 157 rectangle from first 5.775, graph 0 to 5.795, graph 1 fs solid fc rgb "#d0ffd0" back
set label  157 center at first 5.785, graph 0.95 "157" font ",14"

set object 161 rectangle from first 5.795, graph 0 to 5.815, graph 1 fs solid fc rgb "#d0d0ff" back
set label  161 center at first 5.805, graph 0.95 "161" font ",14"

set object 165 rectangle from first 5.815, graph 0 to 5.835, graph 1 fs solid fc rgb "#ffd0d0" back
set label  165 center at first 5.825, graph 0.95 "165" font ",14"

set object 169 rectangle from first 5.835, graph 0 to 5.855, graph 1 fs solid fc rgb "#d0ffd0" back
set label  169 center at first 5.845, graph 0.95 "169" font ",14"

set object 173 rectangle from first 5.855, graph 0 to 5.875, graph 1 fs solid fc rgb "#d0d0ff" back
set label  173 center at first 5.865, graph 0.95 "173" font ",14"

set object 177 rectangle from first 5.875, graph 0 to 5.895, graph 1 fs solid fc rgb "#ffd0d0" back
set label  177 center at first 5.885, graph 0.95 "177" font ",14"

# DB graph.
set ylabel "dB"
set output db_output
plot \
     survey_data using ($1/1e9):2 with dots lc rgb "blue" notitle, \
     NaN with points pt 7 lc rgb "blue" title "Average", \
     "" using ($1/1e9):3 with dots lc rgb "red" notitle, \
     NaN with points pt 7 lc rgb "red" title "Maximum"

# Linear graph.
set ylabel ""
set output power_output
plot \
     survey_data using ($1/1e9):(10**($2/10.0)) with dots lc rgb "blue" notitle, \
     NaN with points pt 7 lc rgb "blue" title "Average", \
     "" using ($1/1e9):(10**($3/10.0)) with dots lc rgb "red" notitle, \
     NaN with points pt 7 lc rgb "red" title "Maximum"
