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
 * QNodePage.java
 *
 * @author Peter Halasz
 */

package net.pengo.propertyEditor;

import javax.swing.JTextPane;

import net.pengo.resource.Resource;



public class QNodePage extends PropertyPage {
    private Resource res;
    
    /** Creates a new instance of SummaryPage */
    public QNodePage(Resource res) {
        super();
        this.res = res;
        build();
    }
    
    public void build() {
        removeAll();
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        //add(new JLabel( "Size (bytes): " + res.getSelectionData().getLength() ));
        
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        JTextPane jtp = new JTextPane();
        String s = "toString(): " + res + "\n\n" +
			"Sinks (dependents): " + res.getSinkCount() + "\n" +
			"Sources: " + res.getSources().length + "\n"; //xxx: list them!
			
        jtp.setText(s) ;
        add(jtp);
    }
    
    public void save() {
        return;
    }
    
    public String toString() {
        return "Dependency Info";
    }
}
