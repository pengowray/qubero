package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.math.BigInteger;

import net.pengo.hexdraw.original.Place;

public class AddressRenderer extends SeperatorRenderer {
    int base;
    private final int lengthInChar = 10;
    //FIXME: proper calc:  lengthInChar = root.getLength().toString(base) + 2; // longest "hex" address can be (as a string), plus two (for a ": ")
    
    public AddressRenderer(FontMetrics fm, int columnCount, boolean render) {
        this(fm, columnCount, render, 16);
    }
    
    public AddressRenderer(FontMetrics fm, int columnCount, boolean render, int base) {
        super(fm, columnCount, render);
        this.base = base;
    }
    
    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
        //// draw address:
        g.setColor(Color.blue);
        FontMetrics fm = g.getFontMetrics();
        String addr = BigInteger.valueOf(lineNumber).toString(base);
        //addr = new String(addr.getBytes(), 0, 8); // right justify ?
        int rightJust = getWidth() - fm.stringWidth(addr);
        g.drawString(addr, rightJust, fm.getAscent() );
        if (addr.length() > lengthInChar) {
            //FIXME: address is too long. truncate or abbrevriate
            System.out.println("address field too long");
        }
    }
    
    public int getWidth() {
        return fm.charWidth('W')*lengthInChar;
    }

    public long whereAmI(int x, int y, long lineAddress){
        return -1;
    }
    
    public String toString() {
        return "Address" + base;
    }

}
