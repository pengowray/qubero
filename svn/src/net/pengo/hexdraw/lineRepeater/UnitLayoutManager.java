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

/**
 * UnitLayoutManager.java
 *
 * @author Peter Halasz
 */

package net.pengo.hexdraw.lineRepeater;

import java.awt.*;
import net.pengo.splash.FontMetricsCache;

class UnitLayoutManager implements LayoutManager { //FIXME: make LayoutManager2
    protected int unitStart[]; // length of array is "units"
    protected int unitBound[]; // start and end of unit, with spacing. length length is units+1.
    protected int maxHeight; // highest component
    
    public void addLayoutComponent(String name, Component comp) {
        // need to call layoutContainer ?
    }
    
    public void layoutContainer(Container parent) {
        calcBounds(parent, true);
    }
    
    public Dimension minimumLayoutSize(Container parent) {
        // should try squishing! (both components and padding/spacing.. give an option for which to do first or how much of each?)
        return preferredLayoutSize(parent);
    }
    
    public Dimension preferredLayoutSize(Container parent) {
        calcBounds(parent, false);
        return (new Dimension(unitBound[unitBound.length-1], maxHeight));
    }
    
    public void removeLayoutComponent(Component comp) {
    }
    
    // doLayout true to actually change the layout. false to just calculate where everything should be.
    // unitBound is currently ignored. (except for gross size) Should be used to create a nice box around each unit/component.
    protected void calcBounds(Container parent, boolean doLayout) {
        int units = parent.getComponentCount();
        //int unitWidth = (int)unitPainter.getPreferredSize().getWidth(); //FIXME: precision loss. double->int
        int spacingWidth = FontMetricsCache.singleton().getFontMetrics("hex").charWidth('W'); // FIXME: cache this
        //int spacingWidth = unitWidth;
        
        float align = .5f; // .5 == put half of each space to the left side of the unit
        int leftPad = spacingWidth; // how much to pad the left
        int rightPad = spacingWidth;

        if (units == 0) {
            unitStart = new int[] { leftPad } ;
            unitBound = new int[] { 0, leftPad + rightPad };
            maxHeight = 0;
            return;
        }
        
        unitStart = new int[units];
        unitBound = new int[units + 1]; // start and end of unit, with spacing
        
        int[] spaceEvery = new int[] {1, 2, 4, 8}; //FIXME: use denominators
        
        int[] spaceSize = new int[] {spacingWidth, 0, spacingWidth, spacingWidth}; // original
        //int[] spaceSize = new int[] {spacingWidth/2, spacingWidth/2, spacingWidth, spacingWidth};
        
        
        int s = 0;
        unitBound[0] = 0;
        unitStart[0] = leftPad;
        Component c = parent.getComponent(0);
        if (doLayout) c.setSize(c.getPreferredSize());
        if (doLayout) c.setLocation(leftPad, 0);
        maxHeight = c.getHeight();
        
        for (int n=1; n<units; n++) {
            //hexStart[n] = s;
                
            int gap = 0;
            for (int m=0; m<spaceEvery.length; m++) {
                if ((n+1) % spaceEvery[m] == 0) {
                    gap += spaceSize[m];
                }
            }
            Component c0 = parent.getComponent(n-1);
            Component c1 = parent.getComponent(n);
            
            unitStart[n] = unitStart[n-1] + c0.getPreferredSize().width + gap;
            unitBound[n] = unitStart[n-1] + c0.getPreferredSize().width + (int)(gap * align);
            
            if (doLayout) c1.setSize(c1.getPreferredSize());
            if (doLayout) c1.setLocation(unitStart[n], 0);
            
            maxHeight = Math.max(maxHeight, c.getHeight());
            
        }
        unitBound[units] = unitBound[units-1] + parent.getComponent(units-1).getWidth() + rightPad;
    }
    
}

