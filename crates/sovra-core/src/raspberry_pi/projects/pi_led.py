from gpiozero import LED
from time import sleep

# GPIO number 17 means physical pin 11.
with LED(17, initial_value=False) as light:
    for _ in range(10):
        light.on()
        sleep(0.5)
        light.off()
        sleep(0.5)
