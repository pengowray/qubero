package net.pengo.hexdraw.lineRepeater;

import java.awt.*;
import javax.swing.JPanel;

class LinePanel extends JPanel {
    protected byte[] bytes; // currently drawing this.
    protected int offset;
    
    /** Creates a new instance of LinePanel */
    public LinePanel() {
        super();
        init();
    }
    public LinePanel(boolean isDoubleBuffered) {
        super(isDoubleBuffered);
        init();
    }
    public LinePanel(LayoutManager layout) {
        super(layout);
        init();
    }
    public LinePanel(LayoutManager layout, boolean isDoubleBuffered) {
        super(layout, isDoubleBuffered);
        init();
    }
    private void init() {
    }

    public void paint(Graphics g, byte[] bytes, int offset) {
        //FIXME: flawed function. sync problems.
        this.bytes = bytes;
        this.offset = offset;
        paint(g);
    }
    
    public void paint(Graphics g) {
        super.paint(g);
        Rectangle r = g.getClipBounds();
        g.setColor(Color.BLUE);
        g.drawLine(r.x, r.y, r.x+r.width, r.y+r.height);
    }
    
    public byte getByte(int offset) {
        return bytes[this.offset + offset];
    }
    
    public UnitInfo getUnitInfo(int offset) {
        return new UnitInfo(getByte(offset), UnitInfo.NORMAL);
    }
        /** calculate size this would be painted at? */
    public Dimension paintSize(byte[] bytes, int offset) {
        return getSize();
    }
}

