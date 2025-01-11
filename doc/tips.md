## Useful commands

### Plot I/Q data

```
$ od -A none -w8 -f test.c32 > t
$ gnuplot
gnuplot> plot 't' using 1 w l, 't' using 2 w l
```
