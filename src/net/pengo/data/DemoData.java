package net.pengo.data;


import java.awt.Image;
import java.awt.image.PixelGrabber;
import java.util.Random;

/**
 * rudimentary file editing. no extending file size.
 * entire file is cached.
 */
public class DemoData extends ArrayData {
    
    private Image logo;
    
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
    
    public void addData(String addition) {
        byte[] string = addition.getBytes();
        int line = 16;
        
        //round up to nearest 16 bytes
        if (string.length % line != 0) {
            byte[] tabbed = new byte[string.length + (line - (string.length % line ))];
            System.arraycopy(string, 0, tabbed, 0, string.length);
            addData(tabbed);
        } else {
            addData(string);
        }
    }
    
    public void addData(byte[] addition) {
        byte[] newBytes = new byte[byteArray.length + addition.length];
        System.arraycopy(byteArray, 0, newBytes, 0, byteArray.length);
        System.arraycopy(addition, 0, newBytes, byteArray.length, addition.length);
        byteArray = newBytes;
    }
    
    public DemoData(Image logo) {
        super();
        this.logo = logo;
        byteArray = new byte[0];
        
        
        addData("Qubero Demo File");
        addData("4-Bit Spectrum");
        addData(hexSpectrum());
        addData(hexSpectrum());
        addData("1-Byte Spectrum");
        addData(oneByteSpectrum());
        addData("32-bit integers counter");
        addData(fourByteSpectrum());
        addData("Random data");
        addData(randomData(256));
        addData("Logo");
        addData(logo());
        addData("Qubero (c) 2002-2004 Peter Halasz");
        
        //int length = header.length + 1024 + footer.length;
        
        
        // copy message into the middle
        //System.arraycopy(msg, 0, byteArray, (256-msg.length)/2, msg.length); // actual middle
        
        
        
        // logo
        
        
    
    //System.arraycopy(boiler, 0, byteArray, byteArray.length - boiler.length, boiler.length);
}

private byte[] hexSpectrum() {
    //return new Long(0x01234567890abcdefL). // convert to byte array?.. 
    
    //hmm. just hard code i guess
    //return new byte[]{0x00,0x12,0x34,0x56,0x78,0x90,0xAB,0xCD,0xEF};
    return new byte[]{(byte)0x01,(byte)0x23,(byte)0x45,(byte)0x67,(byte)0x89,(byte)0xAB,(byte)0xCD,(byte)0xEF};
}

private byte[] oneByteSpectrum() {
    byte[] spec = new byte[256];
    
    byte b = 0;
    for (int i=0; i<spec.length; i++) {
        spec[i] = (byte)b;
        b++;
    }
    
    return spec;
}

private byte[] fourByteSpectrum() {
    Random random = new Random();
    byte[] spec = new byte[256];
    
    int num = random.nextInt();
    for (int i=0; i<spec.length; i+=4) {
        int a   = (num >> 24) & 0xff;
        int b   = (num >> 16) & 0xff;
        int c   = (num >>  8) & 0xff;
        int d   = (num      ) & 0xff;
        spec[i  ] = (byte)a;
        spec[i+1] = (byte)b;
        spec[i+2] = (byte)c;
        spec[i+3] = (byte)d;
        
        num++;
    }
    
    return spec;
}

private byte[] randomData(int len) {
    Random random = new Random();
    byte[] rbytes = new byte[len];
    random.nextBytes(rbytes);
    
    return rbytes;
}

public String toString() {
    return "Demo" + byteArray.length;
}

private byte[] logo() {
    if (logo == null)
        return new byte[0];
    
    int offset = 0;
    byte[] returnBytes = new byte[256]; //fixme: can be filled with other stuff
    
    int w = 16, h = 16;
    int[] pixels = new int[w * h];
    
    Image i = logo.getScaledInstance(w,h,Image.SCALE_SMOOTH);
    PixelGrabber pg = new PixelGrabber(i,0,0,w,h,pixels,0,w);
    try {
        pg.grabPixels();
    } catch (InterruptedException e) {
        System.out.println("logo failure.");
        return new byte[0];
    }
    
    for (int y=0; y<16; y++) {
        for (int x=0; x<16; x++) {
            int nowOff = y * w + x;
            int pixel = pixels[nowOff];
            byte old = returnBytes[offset + nowOff];
            byte alpha = (byte) ((pixel >> 24) & 0xff);
            int red   = (pixel >> 16) & 0xff;
            int green = (pixel >>  8) & 0xff;
            int blue  = (pixel      ) & 0xff;
            byte grey = (byte) (((red+green+blue)/3) & 0xff);
            //byte newPixel = (byte) ((old & ~(~alpha & grey)));
            byte newPixel = (byte)( (old & ~alpha) | (~grey & alpha)  );
            
            returnBytes[offset + nowOff] = newPixel;
        }
    }
    return returnBytes;
    
    
}

}