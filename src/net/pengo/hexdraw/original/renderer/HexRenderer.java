package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.text.NumberFormat;

import net.pengo.hexdraw.original.Place;

public class HexRenderer extends SeperatorRenderer {
    
    //FIXME: allow setting of groupings widths
    
    int[] hexStart; // cached/lazy
    int base;
    boolean zeropad = true; // pad short numbers with 0's
    
    public HexRenderer(FontMetrics fm, int columnCount, boolean render, int base, boolean zeropad) {
        super(fm,columnCount,render);
        setBase(base);
        setZeropad(zeropad);
    }
        
    public HexRenderer(FontMetrics fm, int columnCount, boolean render) {
        this(fm,columnCount,render,16,true);
    }
    
    public void setBase(int base) {
        this.base = base;
    }
    
    public void setZeropad(boolean zeropad) {
        this.zeropad = zeropad;
    }
    
    // longest length for a byte "symbol"
    private int maxChars() {
        int unsigned = 0xff; // highest value
        return Integer.toString(unsigned, base).length();
    }

    //FIXME: make this a static lookup
    private String byte2hex(byte b) {
        int unsigned = ((int)b & 0xff);
        String hex = Integer.toString(unsigned, base);
        
        int padlen = maxChars() - hex.length(); 
        for (int i=0; i<padlen; i++){
            if (zeropad) {
                hex = "0"+hex;
            } else {
                hex = " "+hex;
            }
        }
        
        return hex;
    }
        
//    private static String byte2hex(byte b)  {
//        int l = ((int)b & 0xf0) >> 4;
//        int r = (int)b & 0x0f;
//        
//        return ""
//        	+ (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
//        	+ (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
//    }
    
    public int[] hexStart() {
        if (hexStart == null) {
            int charWidth = fm.charWidth('W');
	        int[] spaceEvery = new int[] {1, 2, 4, 8};
	        int[] spaceSize = new int[]  {charWidth*(maxChars()+1), 0, charWidth, charWidth };
	        
	        hexStart = new int[columnCount+1]; // where each hex starts on a line + where ascii starts
	        
	        int s = 0;
	        for (int n=0; n<hexStart.length; n++) { // only place hexStart.length should be used. use hexPerLine instead.
	            hexStart[n] = s;
	            for (int m=0; m<spaceEvery.length; m++)  {
	                if ((n+1) % spaceEvery[m] == 0)  {
	                    s += spaceSize[m];
	                }
	            }
	        }
        }
        
        return hexStart;
    }
    public void renderBytes( Graphics g, long lineNumber,
            				byte ba[], int baOffset, int baLength, boolean selecta[],
            				Place cursor) {
        //// draw hex:

        int charsHeight = fm.getHeight();
        int charsWidth = fm.charWidth('W') * maxChars();
        int[] hexStart = hexStart();
        
        for ( int i=0; i < baLength; i++ ) {
            if (selecta[i+baOffset])  {
                g.setColor( new Color(170,170,255) );
                
                // fill in gaps between text e.g. "FF (here) FF"
                int fillWidth = (i+1 < baLength && selecta[i+baOffset+1]) ? hexStart[i+1] - hexStart[i] : charsWidth; 
                g.fillRect( hexStart[i], 0, fillWidth, charsHeight);
            }
            
            g.setColor(Color.black);
            g.drawString( byte2hex(ba[baOffset+i]), hexStart[i], fm.getAscent());
            
        }
    }
    
    

    public int getWidth() {
        int charsWidth = fm.charWidth('W') * maxChars();
        int[] hexStart = hexStart();
        
        return hexStart[columnCount-1] + charsWidth;
    }
    
    public void setColumnCount(int cc) {
        hexStart = null; // reset for recalc
        super.setColumnCount(cc);
    }
    public void setFontMetrics(FontMetrics fm) {
        hexStart = null; // reset for recalc
        super.setFontMetrics(fm);
    }
    
    public long whereAmI(int x, int y, long lineAddress){
        int[] hs = hexStart();
        for (int i = 0; i < hs.length; i++) {
            int start = hs[i];
            if (x < start)
                return lineAddress+i-1;
        }
        return -1;
    }
    
    public String toString() {
        if (base==16)
            return "Hex";
        
        if (base==10)
            return "Decimal";
        
        if (base==8)
            return "Octal";
        
        if (base==2)
            return "Binary";

        return "base " + base;
    }

}
