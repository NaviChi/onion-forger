import math
from PIL import Image, ImageDraw

def create_spinner_frame(size, angle):
    # Create an RGBA image with full transparency
    img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    
    # Calculate center and radius
    center = (size / 2, size / 2)
    radius = size * 0.4
    
    # Draw a smooth, gradient-like arc using multiple circles for a glowing effect
    for i in range(10):
        alpha = int(255 * (1 - i/10))
        # Vibe Architecture Colors: Neon Cyan / "Military-Grade" accents
        color = (0, 255, 170, alpha)
        
        # Calculate start and end angles
        start_angle = angle - 60
        end_angle = angle + 60
        
        # Draw arc segments 
        d.arc([center[0]-radius+i, center[1]-radius+i, 
               center[0]+radius-i, center[1]+radius-i], 
              start=start_angle, end=end_angle, fill=color, width=4)
              
    return img

def main():
    print("Generating SnoozeSlayer Vibe Architecture 60fps WebP Spinner...")
    
    size = 120  # High-res master source
    frames = []
    
    # Generate 60 frames for a fully smooth 60fps cinematic loop
    num_frames = 60
    for i in range(num_frames):
        # Rotate complete 360 degrees
        angle = (i / num_frames) * 360
        frame = create_spinner_frame(size, angle)
        frames.append(frame)
        
    # Save as highly optimized 8-bit animated WebP with true alpha
    # 60fps = ~16ms duration per frame
    output_path = "public/assets/animations/vibe_spinner_8bit_alpha.webp"
    frames[0].save(
        output_path, 
        format="WebP",
        save_all=True,
        append_images=frames[1:],
        duration=16, 
        loop=0,
        lossless=True,
        quality=100
    )
    
    print(f"✅ Successfully exported Masterpiece Asset: {output_path}")

if __name__ == "__main__":
    main()
