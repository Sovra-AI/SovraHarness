from machine import Pin
from time import sleep

# GPIO number 15 means physical pin 20.
light = Pin(15, Pin.OUT, value=0)
try:
    for _ in range(10):
        light.on()
        sleep(0.5)
        light.off()
        sleep(0.5)
finally:
    light.off()
