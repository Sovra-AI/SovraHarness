from gpiozero import Button
from time import monotonic, sleep

with Button(17, pull_up=True, bounce_time=0.05) as button:
    end = monotonic() + 30
    previous = None
    while monotonic() < end:
        pressed = button.is_pressed
        if pressed != previous:
            print('Pressed' if pressed else 'Released')
            previous = pressed
        sleep(0.05)
print('Finished')
