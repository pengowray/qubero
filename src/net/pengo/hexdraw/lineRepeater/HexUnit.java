/**
 * HexUnit.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.hexdraw.lineRepeater;

import java.awt.*;
import net.pengo.splash.FontMetricsCache;

class HexUnit extends Component {
    protected FontMetrics fm;
    protected Dimension prefSize;
    protected int offset; // used when asking parent for a unit.
    
    //FIXME: getHowManyPrevAndForwardBytesNeededToDraw ?
    
    public HexUnit(int offset){
        super();
        this.offset = offset;
        setFont(FontMetricsCache.singleton().getFont("hex"));
        fm = FontMetricsCache.singleton().getFontMetrics("hex");
    }
    
    public void paint(Graphics g) {

        Container con = getParent();
        if (con instanceof LinePanel) {
            LinePanel linpan = (LinePanel)con;
            paint(g, linpan.getUnitInfo(offset));
        }
        // else do nothing
    
        ////
        Rectangle r = g.getClipBounds();
        g.setColor(Color.RED);
        g.drawLine(r.x, r.y, r.x+r.width, r.y+r.height);
        System.out.println("drawing " + offset);
        ////
    }

    public void paint(Graphics g, UnitInfo unit) {
        
        if (unit.getState() == unit.EMPTY)
            return;
        
        String str = unit.hexString();
        g.drawString(str, 0, fm.getAscent());
        System.out.println("drawing " + str); // xxx
    }
    
    public Dimension getPreferredSize() {
        if (prefSize == null) {
            prefSize = new Dimension(fm.getMaxAdvance() * 2, fm.getHeight());
        }
        return prefSize;
    }
    
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }
    
    public Dimension getMaximumSize() {
        // allow this to be bigger maybe?
        return getPreferredSize();
    }
    
}

