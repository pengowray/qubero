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
 * Created on July 11, 2003, 11:51 PM
 */

package net.pengo.propertyEditor;

import javax.swing.JLabel;
import javax.swing.JTextField;

import net.pengo.resource.Resource;
import net.pengo.resource.StringResource;
/**
 *
 * @author  Peter Halasz
 */
public class StringPage extends EditablePage {
    private StringResource res;
    private JTextField inputField;
    
     public StringPage(StringResource res) {
         this(res, null);
     }
     
    public StringPage(StringResource res, PropertiesForm form) {
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
    	//System.out.println("Text saveOp");
        String val = inputField.getText();
        res.setValue(val);
    }
    
    public void buildOp() {
        String val = res.getValue();
        if (val==null || val.equals("")) {
            inputField.setText("");
        } else {
            inputField.setText(val);
        }
    }
    
    public boolean isValid() {
        //fixme
        return true; // success
    }
    
    public String toString() {
        return "Name";
    }
}
