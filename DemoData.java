import java.io.*;
import java.util.Random;
import java.awt.image.*;
import java.awt.*;

/** 
 * rudimentary file editing. no extending file size.
 * entire file is cached.
 */
class DemoData extends Data {
    protected byte[] data;

    public DemoData() {
        this(null);
    }
    
    public DemoData(int length) {
       // stripes
        data = new byte[length];
        Random random = new Random();
        
        random.nextBytes(data);
        data[length-1] = (byte)0xff;
        data[0] = 0x00;
    }
    
    public DemoData(Image logo) {
        byte[] msg = "welcome to mooj!".getBytes();
        byte[] boiler = "Mooj (c) 2002 Peter Halasz".getBytes();
        int length = 1024 + boiler.length;

	data = new byte[length];
        
        byte b = 0;
        for (int a=0; a < length; a++) {
            data[a] = b;
            b++;
        }
        
        Random random = new Random();
        byte[] rbytes = new byte[256];
        random.nextBytes(rbytes);
        System.arraycopy(rbytes, 0, data, 256*2, rbytes.length);
        
        // copy message into the middle
        //System.arraycopy(msg, 0, data, (256-msg.length)/2, msg.length); // actual middle
        System.arraycopy(msg, 0, data, 128, msg.length);
        
        
        
        int offset = 768;

        // stripes
        byte[] st = new byte[16];
        random.nextBytes(st);
        for (int ay = 0; ay < 16; ay++) {
            for (int ax = 0; ax < 16; ax++) {
                data[offset + ay*16 + ax] = st[ay];
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
                    byte old = data[offset + nowOff];
                    byte alpha = (byte) ((pixel >> 24) & 0xff);
                    int red   = (pixel >> 16) & 0xff;
                    int green = (pixel >>  8) & 0xff;
                    int blue  = (pixel      ) & 0xff;
                    byte grey = (byte) (((red+green+blue)/3) & 0xff); 
                    //byte newPixel = (byte) ((old & ~(~alpha & grey)));
                    byte newPixel = (byte)( (old & ~alpha) | (~grey & alpha)  );

                    data[offset + nowOff] = newPixel;
                }
            }
        }
        
        System.arraycopy(boiler, 0, data, data.length - boiler.length, boiler.length);
    }

    
    public InputStream getDataStream(long start, long length) {
        return new ByteArrayInputStream(data, (int)start, (int)length); // no loss.
    }

    public InputStream getDataStream(long offset) {
        return new ByteArrayInputStream(data, (int)offset, data.length-(int)offset); // no loss.
    }

    public InputStream getDataStream(){
        return new ByteArrayInputStream(data);
    }
    
    public long getLength() {
        return data.length;
    }

    
    public String toString() {
        return "Demo" + data.length;
    }

    public String getType() {
        return "demo";
    }
    
}
