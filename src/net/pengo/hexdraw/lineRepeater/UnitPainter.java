/**
 * UnitPainter.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.hexdraw.lineRepeater;

/** paint a single "unit". all components within the unit draw the same value. */
import java.awt.*;

class UnitPainter extends Container {
    public void addImpl(Component comp, Object constraints, int index) {
        // check that component is a unit-ish component?
        // or wrap it in a UnitPainter
        super.addImpl(comp,constraints,index);
    }
    
    public void paintComponents(Graphics g, UnitInfo unit) {
        Component[] com = getComponents();
        for (int i=0; i<com.length; i++) {
            if (com[i] instanceof HexUnit) { // xxx HexUnit or another class ???
                ((HexUnit)com[i]).paint(g, unit);
            } else {
                com[i].paint(g);
            }
        }
    }
}


