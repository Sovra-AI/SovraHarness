from machine import Pin
from time import ticks_ms, ticks_diff, sleep_ms

button = Pin(14, Pin.IN, Pin.PULL_UP)
start = ticks_ms()
previous = None
while ticks_diff(ticks_ms(), start) < 30000:
    pressed = button.value() == 0
    sleep_ms(50)
    if pressed == (button.value() == 0) and pressed != previous:
        print('Pressed' if pressed else 'Released')
        previous = pressed
print('Finished')
