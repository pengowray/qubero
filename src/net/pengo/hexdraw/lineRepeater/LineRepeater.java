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

/*
 * LinePanel.java
 *
 * Created on 11 September 2002, 10:02
 */

// attempt to create a custom way of redisplaying a panel (a line of hex)
// without recreating it (i.e. just using one). didn't work. too fiddly.

package net.pengo.hexdraw.lineRepeater;

import net.pengo.data.*;

import javax.swing.*;
import java.awt.*;
import java.io.IOException;

/**
 *
 * @author  Peter Halasz
 */
public class LineRepeater extends JPanel {
    protected Data datasource;
    
    protected LinePanel linePanel;
    protected int units = 16; // default units in a line
    protected int width;
    protected int height; //FIXME: too short?
    
    public LineRepeater(Data datasource) {
        this.datasource = datasource;
        linePanel = new LinePanel();
        //linePanel.setLayout(new UnitLayoutManager()); //FIXME: uncomment when i know it works? :)
        linePanel.setLayout(new FlowLayout());
        for (int i=0; i<units; i++) {
            linePanel.add(new HexUnit(i));
        }
        linePanel.setSize(linePanel.getPreferredSize()); //FIXME: probably not neccessary ?
        System.out.println("line panel size: " + linePanel.getPreferredSize());
        System.out.println("line panel components: " + linePanel.getComponentCount());

        width = linePanel.getWidth();
        height = linePanel.getHeight() * (int)(datasource.getLength() / units);

        add(linePanel);
    }
    
    public void paintComponent(Graphics g) {
        super.paintComponent(g);
        Rectangle clip = g.getClipBounds();
        
        int lineHeight = linePanel.getHeight();
        int firstLine = clip.y / lineHeight;
        int lastLine = (clip.y + clip.height) / lineHeight +1; //FIXME: +1 sometimes
        int dataShowStart = (clip.height / height) * units;
        int dataShowLength = (int)datasource.getLength(); //FIXME: more than shown, but at least not more than the data. //FIXME: loss of precision
		
		try
		{
			byte[] d = datasource.subdata(dataShowStart,dataShowLength).readByteArray();
			
			for (int line=firstLine; line < lastLine; line++)
			{
				Graphics gm = g.create(0, line*lineHeight, clip.width, lineHeight);
				linePanel.setLocation(0, line*units);
				linePanel.paint(gm, d, line*units); //FIXME: will have sync problems. need multiple linePanels
			}
		}
		catch (IOException e) {
			// now what?
		}
    }
    
    public Dimension getPreferredSize() {
        return new Dimension(width, height);
    }
    
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }
    
    public Dimension getMaximumSize() {
        // allow this to be bigger maybe?
        return getPreferredSize();
    }
}






