/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

package net.pengo.data;


import java.awt.Image;
import java.awt.image.PixelGrabber;
import java.math.BigInteger;
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
        setByteArray(new byte[length]);
        Random random = new Random();
        
        random.nextBytes(getByteArray());
        getByteArray()[length-1] = (byte)0xff;
        getByteArray()[0] = 0x00;
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
    	byte[] byteArray = getByteArray();
        byte[] newBytes = new byte[byteArray.length + addition.length];
        System.arraycopy(byteArray, 0, newBytes, 0, byteArray.length);
        System.arraycopy(addition, 0, newBytes, byteArray.length, addition.length);
        setByteArray(newBytes);
    }
    
    public DemoData(Image logo) {
        super();
        this.logo = logo;
        setByteArray(new byte[0]);
        
        
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
        addData("Binary digits of pi: three point"); // Sample digits for hexa decimal digits of pi (excludes the 3)
        addData(pi());
        addData("Logo");
        addData(logo());
        addData("Qubero (c) 2002-2005 Peter Halasz".getBytes());
        
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
        return "Demo" + getByteArray().length;
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
    
    private byte[] pi() {
        // from http://www.super-computing.org/pi-hexa_current.html
        
        // Sample digits for hexa decimal digits of pi
        
        String pi = // 3.
        "243F6A8885A308D313198A2E03707344A4093822299F31D008" +
        "2EFA98EC4E6C89452821E638D01377BE5466CF34E90C6CC0AC" +
        "29B7C97C50DD3F84D5B5B54709179216D5D98979FB1BD1310B" +
        "A698DFB5AC2FFD72DBD01ADFB7B8E1AFED6A267E96BA7C9045" +
        "F12C7F9924A19947B3916CF70801F2E2858EFC16636920D871" +
        "574E69A458FEA3F4933D7E0D95748F728EB658718BCD588215" +
        "4AEE7B54A41DC25A59B59C30D5392AF26013C5D1B023286085" +
        "F0CA417918B8DB38EF8E79DCB0603A180E6C9E0E8BB01E8A3E" +
        "D71577C1BD314B2778AF2FDA55605C60E65525F3AA55AB9457" +
        "48986263E8144055CA396A2AAB10B6"; // "48986263E8144055CA396A2AAB10B6B4CC5C341141E8CEA154";
        
        return new BigInteger(pi,16).toByteArray();
        
    }
    
}