package net.pengo.data;


import java.awt.Image;
import java.awt.image.PixelGrabber;
import java.util.Random;

/** 
 * rudimentary file editing. no extending file size.
 * entire file is cached.
 */
public class DemoData extends ArrayData {

    public DemoData() {
        this(null);
    }
    
    public DemoData(int length) {
       // stripes
        byteArray = new byte[length];
        Random random = new Random();
        
        random.nextBytes(byteArray);
        byteArray[length-1] = (byte)0xff;
        byteArray[0] = 0x00;
    }
    
    public DemoData(Image logo) {
        super();
        byte[] msg = "Welcome to Qubero!".getBytes();
        byte[] boiler = "Qubero (c) 2002-2004 Peter Halasz".getBytes();
        int length = 1024 + boiler.length;

	byteArray = new byte[length];
        
        byte b = 0;
        for (int a=0; a < length; a++) {
            byteArray[a] = b;
            b++;
        }
        
        Random random = new Random();
        byte[] rbytes = new byte[256];
        random.nextBytes(rbytes);
        System.arraycopy(rbytes, 0, byteArray, 256*2, rbytes.length);
        
        // copy message into the middle
        //System.arraycopy(msg, 0, byteArray, (256-msg.length)/2, msg.length); // actual middle
        System.arraycopy(msg, 0, byteArray, 128, msg.length);
        
        
        
        int offset = 768;

        // stripes
        byte[] st = new byte[16];
        random.nextBytes(st);
        for (int ay = 0; ay < 16; ay++) {
            for (int ax = 0; ax < 16; ax++) {
                byteArray[offset + ay*16 + ax] = st[ay];
            }
        }

        
        // logo
        offset = random.nextInt(48)*16;
        
        if (logo != null) {
            int w = 16, h = 16;
            int[] pixels = new int[w * h];
            
            Image i = logo.getScaledInstance(w,h,Image.SCALE_SMOOTH);
            PixelGrabber pg = new PixelGrabber(i,0,0,w,h,pixels,0,w);
            try {
                pg.grabPixels();
            } catch (InterruptedException e) {
                System.out.println("logo failure.");
                return;
            }
            
            for (int y=0; y<16; y++) {
                for (int x=0; x<16; x++) {
                    int nowOff = y * w + x;
                    int pixel = pixels[nowOff];
                    byte old = byteArray[offset + nowOff];
                    byte alpha = (byte) ((pixel >> 24) & 0xff);
                    int red   = (pixel >> 16) & 0xff;
                    int green = (pixel >>  8) & 0xff;
                    int blue  = (pixel      ) & 0xff;
                    byte grey = (byte) (((red+green+blue)/3) & 0xff); 
                    //byte newPixel = (byte) ((old & ~(~alpha & grey)));
                    byte newPixel = (byte)( (old & ~alpha) | (~grey & alpha)  );

                    byteArray[offset + nowOff] = newPixel;
                }
            }
        }
        
        System.arraycopy(boiler, 0, byteArray, byteArray.length - boiler.length, boiler.length);
    }

    public String toString() {
        return "Demo" + byteArray.length;
    }

}
