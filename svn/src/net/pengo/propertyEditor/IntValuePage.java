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
 * ValuePage.java
 *
 * Used when editing IntegerResources (including IntPrimativeResource and
 * IntAddressedResource).
 *
 * Created on July 11, 2003, 11:51 PM
 */

package net.pengo.propertyEditor;

import java.io.IOException;

import javax.swing.JLabel;
import javax.swing.JTextField;

import net.pengo.resource.IntResource;
/**
 *
 * @author  Peter Halasz
 */
public class IntValuePage extends EditablePage {
    private IntResource res;
    private JTextField inputField;
    
    /** Creates a new instance of ValuePage */
    public IntValuePage(IntResource res) {
        this(res, null);
    }
    
    public IntValuePage(IntResource res, PropertiesForm form) {
        super(form);
        this.res = res;
        this.form = form;

        add(new JLabel( "Value: " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
            
        add(inputField);
        build();
    }
    
    
    public void saveOp() {
//		System.out.println("saveOp() IntValuePage for: " + res + "...");
        try {
			System.out.println(" - setting value to: " + inputField.getText());
            res.setValue(inputField.getText());
        } catch (IOException e ) {
            //fixme
            e.printStackTrace();
        }
//		System.out.println(" - saveOp() complete.");
    }
    
    public void buildOp() {
		//System.out.println("building IntValuePage for: " + res + "..." );
        inputField.setText( res.getValue() + "" );
		//System.out.println(" - build complete.");
    }
    
    public boolean isValid() {
        //fixme
        return true; // success
    }
    
    public String toString() {
        return "Value";
    }
}
