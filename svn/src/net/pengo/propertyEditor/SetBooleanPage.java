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
 * SetBooleanPage.java
 *
 * Created on August 24, 2003, 7:03 AM
 */

package net.pengo.propertyEditor;

import javax.swing.JComboBox;
import javax.swing.JLabel;

import net.pengo.resource.BooleanResource;

/**
 *
 * @author  Peter Halasz
 */
public class SetBooleanPage extends EditablePage {
    private BooleanResource boolRes;
    private JComboBox type;
    
    /** Creates a new instance of SetBooleanPage */
    public SetBooleanPage(BooleanResource boolRes) {
        this(boolRes, null);
    }
    
    public SetBooleanPage(BooleanResource boolRes, PropertiesForm form) {
        super(form);
        this.boolRes = boolRes;

        add(new JLabel( "Boolean value: " ));
        type = new JComboBox(new String[] {
            "false (or 0)", "true (or 1)"} );
        add(type);
        build(); // build before listener to stop apply being hit
        type.addActionListener( getModActionListener() );
    }
        
    public void saveOp() {
        boolRes.setValue( (type.getSelectedIndex()==1 ? true:false) );
    }
    
    public void buildOp() {
	int selection = (boolRes.getValue() ? 1:0);
	type.setSelectedIndex( selection  );
    }
    
    public String toString() {
        return "Boolean value";
    }
    
}
